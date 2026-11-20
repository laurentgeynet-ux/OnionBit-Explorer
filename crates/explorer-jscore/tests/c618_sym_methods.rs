// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Gate JS-C618 — `RegExp.prototype[@@match]/[@@matchAll]/
//! [@@replace]/[@@search]` + dispatch `String.prototype` spec :
//!
//! - `RegExpExec` dynamique partout : `exec` custom observé,
//!   résultat non-objet non-null → `TypeError`.
//! - `@@match` : `flags` lu par `Get`, `lastIndex` = 0 puis
//!   boucle, `AdvanceStringIndex` sur match vide, `null` si
//!   aucun match global.
//! - `@@matchAll` : `SpeciesConstructor` + `Construct`, iter
//!   **paresseux** — `exec` non appelé avant `next()`.
//! - `@@replace` : execs collectés avant toute capture ;
//!   `GetSubstitution` complet (`$$`/`$&`/`` $` ``/`$'`/`$n`/
//!   `$<nom>`) ; remplacement fonction `«m, caps…, pos, s
//!   [, groups]»`.
//! - `@@search` : `lastIndex` sauvé, mis à 0, **restauré**.
//! - `String.prototype.*` : `RequireObjectCoercible` puis
//!   `GetMethod(@@sym)` **avant** `ToString(this)` ; `split`
//!   limite en `ToUint32` ; `replaceAll`/`matchAll` exigent
//!   `IsRegExp` → `flags` contient `g`.

use explorer_jscore::Engine;

/// Évalue `src` et décrit la valeur produite.
#[track_caller]
fn run(src: &str) -> String {
    let mut e = Engine::new();
    e.install_t262();
    match e.eval(src) {
        Ok(v) => e.describe(v),
        Err(err) => format!("ERR {err}"),
    }
}

/// `@@match` : global → tableau des `Get(result,"0")` ; sans
/// `g` → le résultat exec brut ; aucun match global → `null`.
#[test]
fn match_basics() {
    let out = run("var r=[];\
         r.push(/x/g[Symbol.match]('axbx').join());\
         r.push(/(b)/[Symbol.match]('axb').join());\
         r.push(/z/g[Symbol.match]('ab'));\
         r.join('|')");
    assert_eq!(out, "x,x|b,b|");
}

/// Matches vides : `AdvanceStringIndex` évite la boucle
/// infinie — `(?:)` global sur 'ab' → `['','','']`.
#[test]
fn match_empty_advance() {
    let out = run("var r=[];\
         r.push(/(?:)/g[Symbol.match]('ab').length);\
         r.push(/x*/g[Symbol.match]('ab').join());\
         r.join('|')");
    // `x*` sur 'ab' : tout est match vide → `['','','']`.
    assert_eq!(out, "3|,,");
}

/// `flags` lu par `Get` **avant** `lastIndex = 0` puis exec —
/// accesseurs observables dans l'ordre spec.
#[test]
fn match_flags_getter_order() {
    let out = run("var log=[];\
         var rx={exec:function(){log.push('e');return null},lastIndex:0};\
         Object.defineProperty(rx,'flags',{get:function(){log.push('f');return 'g'}});\
         RegExp.prototype[Symbol.match].call(rx,'ab')+':'+log.join(',')");
    assert_eq!(out, "null:f,e");
}

/// `exec` custom appelé dans la boucle globale ; `lastIndex`
/// écrit par l'algo est relu par exec — contrat observable.
#[test]
fn match_custom_exec() {
    let out = run("var calls=[];\
         var rx={exec:function(s){calls.push(this.lastIndex);\
           var i=s.indexOf('x',this.lastIndex);if(i<0)return null;\
           this.lastIndex=i+1;var m=['x'];m.length=1;return m;},\
           lastIndex:9,flags:'g',global:true,unicode:false};\
         RegExp.prototype[Symbol.match].call(rx,'axbx').join()+':'+calls.join(',')");
    // `global` lu comme PROPRIÉTÉ (corpus es6id) — lastIndex
    // forcé à 0 par l'algo → execs à 0,2,4.
    assert_eq!(out, "x,x:0,2,4");
}

/// `exec` retournant une primitive → `TypeError` (contrat
/// `RegExpExec` — `null` seul est « pas de match »).
#[test]
fn exec_result_contract() {
    let out = run("var r=[];\
         var rx={exec:function(){return 5},lastIndex:0,flags:'g'};\
         try{RegExp.prototype[Symbol.match].call(rx,'a')}catch(e){r.push(e.name)}\
         var rx2={exec:function(){return 'x'},lastIndex:0,flags:''};\
         try{RegExp.prototype[Symbol.match].call(rx2,'a')}catch(e){r.push(e.name)}\
         r.join('|')");
    assert_eq!(out, "TypeError|TypeError");
}

/// `@@matchAll` **paresseux** : `SpeciesConstructor` +
/// `Construct` au call, mais `exec` seulement au `next()` —
/// et `lastIndex` copié depuis le receiver.
#[test]
fn matchall_lazy_species() {
    let out = run("var calls=[];\
         function C(a,b){calls.push('ctor:'+b);\
           this.exec=function(s){calls.push('exec:'+this.lastIndex);\
             var i=s.indexOf('x',this.lastIndex);if(i<0)return null;\
             this.lastIndex=i+1;return {index:i,length:1};};\
           this.lastIndex=0;}\
         C[Symbol.species]=C;\
         var rx=/x/g;rx.constructor=C;rx.lastIndex=1;\
         var it=rx[Symbol.matchAll]('axbx');\
         calls.push('created');\
         var r=[];\
         r.push(it.next().value.index);\
         r.push(it.next().value.index);\
         r.push(it.next().done);\
         r.join(',')+':'+calls.join(',')");
    // execs relisent lastIndex : 1 → 2 → 4 (i+1 écrit par exec).
    assert_eq!(out, "1,3,true:ctor:g,created,exec:1,exec:2,exec:4");
}

/// `@@matchAll` non-global : un seul `next` avec valeur,
/// puis `done` — sans avance `lastIndex` (pas de `g`).
#[test]
fn matchall_non_global() {
    let out = run("var it=/b/[Symbol.matchAll]('abc');\
         var a=it.next();var b=it.next();\
         a.value.index+','+b.done");
    assert_eq!(out, "1,true");
}

/// `@@matchAll` : match vide avance `lastIndex` (global) —
/// `/(?:)/g` sur 'ab' → 3 résultats puis `done`.
#[test]
fn matchall_empty_advance() {
    let out = run("var it=/(?:)/g[Symbol.matchAll]('ab');var n=0;\
         while(!it.next().done)n++;n");
    assert_eq!(out, "3");
}

/// `@@replace` : `GetSubstitution` complet — `$$` `$&` `` $` ``
/// `$'` `$n` `$nn` `$<nom>` — captures `undefined` → "".
#[test]
fn replace_substitutions() {
    let out = run("var r=[];\
         r.push(/b/[Symbol.replace]('abc','X'));\
         r.push(/(b)(c)/[Symbol.replace]('abc','$2$1'));\
         r.push('abc'.replace(/b/,'[$&]$`$\\''));\
         r.push('ab'.replace(/(x)?b/,'<$1>'));\
         r.push('a'.replace(/a/,'$$'));\
         r.push('abc'.replace(/(?<q>b)/,'$<q>-'));\
         r.push('ab'.replace(/b/,'$9$99'));\
         r.join('|')");
    // `'abc'` : `[b]` + $`('a') + $'('c') puis queue 'c' →
    // 'a[b]acc' ; `'ab'` sans 'c' → 'a<>'.
    assert_eq!(out, "aXc|acb|a[b]acc|a<>|$|ab-c|a$9$99");
}

/// Remplacement fonction : arguments `«matched, caps…, pos,
/// string [, groups]»` dans l'ordre ; `groups` transmis.
#[test]
fn replace_function_args() {
    let out = run("var r=[];\
         r.push('a-b'.replace(/(a)-(b)/,function(){return [].join.call(arguments,'/')}));\
         r.push('ab'.replace(/(?<q>b)/g,function(){var g=arguments[arguments.length-1];return 'b/1/ab/'+(typeof g)+':'+(Object.getPrototypeOf(g)===null)+':'+g.q}));\
         r.join('|')");
    // `groups` = ObjectCreate(null) — `String(g)` jetterait
    // (pas de toString hérité) : on sonde type/proto/capture.
    assert_eq!(out, "a-b/a/b/0/a-b|ab/1/ab/object:true:b");
}

/// Boucle globale : execs **collectés** avant les lectures
/// de captures (ordre spec observable) ; match vide avance.
#[test]
fn replace_global_and_empty() {
    let out = run("var r=[];\
         r.push('aXbXc'.replace(/x/gi,function(m,p){return m+p}));\
         r.push('ab'.replace(/(?:)/g,'-'));\
         var log=[];var rx={flags:'g',global:true,unicode:false,lastIndex:0,\
           exec:function(s){log.push('e');var i=s.indexOf('x',this.lastIndex);\
             if(i<0)return null;this.lastIndex=i+1;return {index:i,0:'x',length:1};}};\
         r.push(RegExp.prototype[Symbol.replace].call(rx,'axbx','-')+':'+log.join(','));\
         r.join('|')");
    // 'axbx' : x→- aux index 1 et 3 → 'a-b-' (3 execs : 0,2,4).
    assert_eq!(out, "aX1bX3c|-a-b-|a-b-:e,e,e");
}

/// `@@search` : `lastIndex` sauvé → 0 → exec → **restauré** ;
/// getters/setters observables dans l'ordre spec.
#[test]
fn search_lastindex_restored() {
    let out = run("var log=[];\
         var rx={_li:5,exec:function(){log.push('exec');return {index:2,length:1}}};\
         Object.defineProperty(rx,'lastIndex',{get:function(){log.push('get');return this._li},\
           set:function(v){log.push('set:'+v);this._li=v}});\
         var r=[];\
         r.push(RegExp.prototype[Symbol.search].call(rx,'abc')+':'+log.join(','));\
         var r3=/b/g;r3.lastIndex=5;\
         var i='xbx'.search(r3);\
         r.push(i+','+r3.lastIndex);\
         r.push('abc'.search(/z/));\
         r.join('|')");
    assert_eq!(out, "2:get,set:0,exec,get,set:5|1,5|-1");
}

/// Dispatch `String.prototype` : `GetMethod(@@sym)` sur
/// l'argument **avant** `ToString(this)` — le `toString` du
/// receiver n'est pas appelé quand la méthode custom existe.
#[test]
fn string_dispatch_before_tostring() {
    let out = run("var log=[];\
         var o={toString:function(){log.push('ts');return 'x'}};\
         var sep={\
           [Symbol.match]:function(){log.push('m');return 42},\
           [Symbol.search]:function(){log.push('s');return 7},\
           [Symbol.split]:function(){log.push('p');return 'S'},\
           [Symbol.replace]:function(){log.push('r');return 'R'}};\
         var r=[];\
         r.push(String.prototype.match.call(o,sep));\
         r.push(String.prototype.search.call(o,sep));\
         r.push(String.prototype.split.call(o,sep));\
         r.push(String.prototype.replace.call(o,sep));\
         r.join(',')+':'+log.join(',')");
    assert_eq!(out, "42,7,S,R:m,s,p,r");
}

/// `RequireObjectCoercible` d'abord : `null`/`undefined` →
/// `TypeError` avant tout `GetMethod` sur l'argument.
#[test]
fn string_coercible_first() {
    let out = run("var r=[];\
         var sep={[Symbol.match]:function(){return 1}};\
         try{String.prototype.match.call(null,sep)}catch(e){r.push(e.name)}\
         try{String.prototype.split.call(undefined,sep)}catch(e){r.push(e.name)}\
         try{String.prototype.replace.call(null,sep)}catch(e){r.push(e.name)}\
         r.join('|')");
    assert_eq!(out, "TypeError|TypeError|TypeError");
}

/// `String.prototype.split` : `limit` en `ToUint32` (NaN→0,
/// 2.9→2, -1→2³²-1) ; un vrai `RegExp` route via `@@split`
/// (spec — captures intercalées).
#[test]
fn string_split_spec() {
    let out = run("var r=[];\
         r.push('a-b-c'.split('-',2.9).join());\
         r.push('a-b-c'.split('-',NaN).length);\
         r.push('a-b-c'.split('-',-1).length);\
         r.push('a-b-c'.split(/-/).join());\
         r.push('ab'.split('').join());\
         r.join('|')");
    assert_eq!(out, "a,b|0|3|a,b,c|a,b");
}

/// `replaceAll`/`matchAll` : `IsRegExp` via `Symbol.match`
/// (observable) + `flags` doit contenir `g` — avant tout
/// dispatch `@@replace`/`@@matchAll`.
#[test]
fn replaceall_matchall_global_required() {
    let out = run("var r=[];\
         try{'ab'.replaceAll(/b/,'x')}catch(e){r.push(e.name)}\
         r.push('ab'.replaceAll(/b/g,'x'));\
         var fake={[Symbol.match]:true,flags:'',\
           [Symbol.replace]:function(){return 'R'}};\
         try{'a'.replaceAll(fake,'x')}catch(e){r.push(e.name)}\
         try{'ab'.matchAll(/b/)}catch(e){r.push(e.name)}\
         r.push('ab'.replaceAll('b','x'));\
         r.join('|')");
    assert_eq!(out, "TypeError|ax|TypeError|TypeError|ax");
}

/// `String.prototype.matchAll` : repli `RegExpCreate(rx,'g')`
/// quand l'argument n'a pas `@@matchAll` — iter fonctionnel ;
/// `@@matchAll` custom dispatché sans `ToString(this)`.
#[test]
fn string_matchall_fallback() {
    let out = run("var r=[];\
         var it='aXbXc'.matchAll(/x/gi);\
         var idx=[];var m;\
         while(!(m=it.next()).done)idx.push(m.value.index);\
         r.push(idx.join(','));\
         var log=[];\
         var o={toString:function(){log.push('ts');return 'x'}};\
         var sep={[Symbol.match]:true,flags:'g',\
           [Symbol.matchAll]:function(){log.push('ma');return 'IT'}};\
         r.push(String.prototype.matchAll.call(o,sep));\
         r.push(log.join(','));\
         r.join('|')");
    assert_eq!(out, "1,3|IT|ma");
}
