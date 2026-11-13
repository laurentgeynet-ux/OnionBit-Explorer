// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Verrous JS-C502 — slots d'env `(d,slot)` + `EnvCell::Uninit` :
//! la pré-déclare fige les positions canoniques ; chaque cas
//! doit rendre la valeur exacte que la cellule soit `Bind`,
//! `Uninit` (traitée comme absente) ou hors slot. Cas clés :
//! `x=1` avant `let x` remonte (no TDZ préservé), closures par
//! itération `for(let i)`, paramètre de catch avant les lex du
//! corps, switch partagé, fn-expr nommée, scopes empoisonnés
//! `eval`, kill-switch `EXPLORER_NO_SLOTS`, engagement `slot_hit`.

use explorer_jscore::Engine;

/// Sérialise les tests sensibles au contexte global (env
/// `EXPLORER_NO_SLOTS`, compteurs J17 partagés).
static SLOT_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn eval(src: &str) -> String {
    let mut e = Engine::new();
    match e.eval(src) {
        Ok(v) => e.describe(v),
        Err(err) => panic!("eval: {err}\n  src: {src}"),
    }
}

/// Lecture répétée d'une liaison de bloc — le chemin dominant
/// (`Small[slot]` direct).
#[test]
fn c502_block_let_reads() {
    assert_eq!(
        eval(
            "function f(){let x=3,y=10;var s=0;\
              for(var i=0;i<50;i++)s+=x+y*i;return s}f()"
        ),
        "12400"
    );
}

/// `x=1` avant `let x` dans le même bloc : `Uninit` compte comme
/// absent → l'écriture remonte au var externe (comportement
/// « no TDZ » inchangé).
#[test]
fn c502_assign_before_let_climbs() {
    assert_eq!(
        eval("function f(){var x=0;{x=1;let x=2;};return x}f()"),
        "1"
    );
}

/// Lecture avant `let` dans un bloc imbriqué : la cellule
/// `Uninit` est transparente — le `x` du scope englobant répond.
#[test]
fn c502_read_before_let_climbs() {
    assert_eq!(
        eval("function f(){var x=9;{var r=x;let x=2;return r}return x}f()"),
        "9"
    );
}

/// `let` interne masque un `var` externe : la cellule du slot
/// porte la bonne valeur même si le nom existe plus haut.
#[test]
fn c502_inner_let_shadows_var() {
    assert_eq!(
        eval("function f(){var x=7;{let x=9;var r=x;return r}return x}f()"),
        "9"
    );
}

/// `for(let i)` — les closures de chaque tour capturent des
/// cellules distinctes (les copies d'itération pré-déclarées
/// gardent la même liste canonique).
#[test]
fn c502_for_let_closures() {
    assert_eq!(
        eval(
            "function f(){var fs=[];for(let i=0;i<4;i++)fs.push(()=>i);\
              return fs[0]()+fs[3]()}f()"
        ),
        "3"
    );
}

/// Cible destructurée de `for-of` : slots ordonnés `[a,b]` par
/// itération.
#[test]
fn c502_forof_pattern_slots() {
    assert_eq!(
        eval(
            "function f(){var s='';for(let[a,b]of[[1,2],[3,4],[5,6]])\
              s+=a+'-'+b+'.';return s}f()"
        ),
        "1-2.3-4.5-6."
    );
}

/// Paramètre de catch lié avant les `let` du corps — la liste
/// canonique `[pat…, lex…]` doit être respectée.
#[test]
fn c502_catch_param_then_lex() {
    assert_eq!(
        eval("function f(){try{throw 7}catch(e){let y=e*2;return y+e}return 0}f()"),
        "21"
    );
}

/// `switch` : un bloc déclaratif partagé entre les `case`.
#[test]
fn c502_switch_shared_block() {
    assert_eq!(
        eval(
            "function f(){var r=0;switch(2){case 1:{let x=5;r=x;break}\
              case 2:{let y=8;r=y;break}}return r}f()"
        ),
        "8"
    );
}

/// Function-expression nommée : le nom propre est en slot 0 de
/// l'env `Local` dédiée (récursion interne par ce nom).
#[test]
fn c502_named_fnexpr_self() {
    assert_eq!(
        eval("var f=function rec(n){return n<=1?1:n*rec(n-1)};f(4)"),
        "24"
    );
}

/// Bloc profond — `d>2` garde `resolved_envs`, le slot s'y
/// applique aussi (env cible pré-déclaré).
#[test]
fn c502_deep_block_slots() {
    assert_eq!(
        eval("function f(){{let z=1;{let a=2;{let b=3;{let c=4;return z+a+b+c}}}}return 0}f()"),
        "10"
    );
}

/// Scope `eval`-poisoned : aucune annotation (précédent JS-C490)
/// → la marche dynamique gouverne ; les valeurs restent justes.
#[test]
fn c502_poisoned_scope_dynamic() {
    assert_eq!(eval("function f(){let x=1;eval('x=42');return x}f()"), "42");
}

/// JS-C503 — slots du scope `Local` : paramètres lus en boucle.
#[test]
fn c503_param_slots() {
    assert_eq!(
        eval("function f(a,b){var s=0;for(var i=0;i<40;i++)s+=a+b*i;return s}f(2,3)"),
        "2420"
    );
}

/// `arguments` occupe son slot canonique (après params) — les
/// lectures passent par `LazyArgs`, jamais par le fast-path
/// (`Bind` seul hit) — la valeur reste exacte.
#[test]
fn c503_arguments_position() {
    assert_eq!(
        eval("function f(a){var s=a;for(var i=1;i<arguments.length;i++)s+=arguments[i];return s}f(1,2,3)"),
        "6"
    );
}

/// `new.target` est lié après `arguments` — les noms hoistés
/// après lui gardent leur slot.
#[test]
fn c503_hoist_after_synthetics() {
    assert_eq!(
        eval("function f(){var t=new.target;return ''+h()+t}function h(){return 9}f()"),
        "9undefined"
    );
}

/// Ordre hoist : `var` profonds + `function` déclarés en bloc
/// remontent dans l'ordre de `hoist_vars_only`.
#[test]
fn c503_hoist_order_deep() {
    assert_eq!(
        eval(
            "function f(){var a=1;{var b=2;if(a){var c=3}}\
              while(a<0){var d=4}return a*100+b*10+c}f()"
        ),
        "123"
    );
}

/// `let` de niveau corps = slots en fin de liste canonique.
#[test]
fn c503_body_lex_slots() {
    assert_eq!(
        eval("function f(p){var v=p*2;let x=v+1;const y=x*2;return x+y}f(3)"),
        "21"
    );
}

/// Défauts de paramètres : le nom lié occupe le même slot que
/// son motif, qu'il soit passé ou défaut.
#[test]
fn c503_default_params() {
    assert_eq!(
        eval("function f(a,b=a+1,c){var s=0;for(var i=0;i<3;i++)s+=a+b;return s}f(2)"),
        "15"
    );
}

/// Gen : corps baissé → scope `Local` depth-only, mais les
/// valeurs (et les slots des blocs internes) restent exacts.
#[test]
fn c503_gen_correctness() {
    assert_eq!(
        eval(
            "function*g(){let x=1;{let y=2;yield x+y}var z=5;yield z}\
              var it=g(),s=0,r;while(!(r=it.next()).done)s+=r.value;s"
        ),
        "8"
    );
}

/// JS-C503b — écriture par slot : `x += …`/`x = …` sur une
/// liaison mutable passe par `Small[slot]` direct.
#[test]
fn c503b_write_slots() {
    assert_eq!(
        eval("function f(){let x=0;for(var i=0;i<50;i++)x+=i;return x}f()"),
        "1225"
    );
}

/// Écriture sur `const` : le slot refuse (`mutable=false`) →
/// `env_set` garde le `TypeError` spec.
#[test]
fn c503b_const_write_throws() {
    assert_eq!(
        eval("function f(){const x=1;try{x=2}catch(e){return 'thrown'}return 'no'}f()"),
        "thrown"
    );
}

/// `x = 1` avant `let x` : `Uninit` au slot → repli `env_set`
/// → remontée au `var` externe (no TDZ préservé en écriture).
#[test]
fn c503b_write_before_let_climbs() {
    assert_eq!(
        eval("function f(){var x=0;{x=1;let x=2;return x}return x}f()"),
        "2"
    );
}

/// Cible `for-of` sur variable externe : le site `ForTarget::
/// Expr` est annoté → écriture résolue, valeur exacte.
#[test]
fn c503b_forof_target_write() {
    assert_eq!(
        eval("function f(){var x=0;for(x of[1,2,3]);return x}f()"),
        "3"
    );
}

/// `slot_hit` s'engage sur les lectures de liaisons de bloc.
#[test]
fn c502_slot_hit_engages() {
    let _g = SLOT_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    explorer_jscore::j17_enable();
    let before = explorer_jscore::j17_snapshot();
    let get = |snap: &str| {
        snap.split(' ')
            .find_map(|p| {
                p.strip_prefix("slot_hit=")
                    .and_then(|v| v.parse::<u64>().ok())
            })
            .unwrap_or(0)
    };
    let mut e = Engine::new();
    // Corps de fonction : le resolver n'annote que les `FnLit`
    // matérialisés — le script top-level reste sur la marche.
    let r = e
        .eval(
            "function f(){let x=7;var s=0;\
               for(var i=0;i<2000;i++){let q=x;s+=q}return s}f()",
        )
        .expect("eval");
    assert_eq!(e.describe(r), "14000");
    let after = explorer_jscore::j17_snapshot();
    assert!(
        get(&after) >= get(&before) + 500,
        "slot_hit n'a pas engagé : {before} → {after}"
    );
}

/// `EXPLORER_NO_SLOTS=1` : valeurs identiques, aucune pré-déclare
/// — le slot path est un pur raccourci.
#[test]
fn c502_no_slots_killswitch() {
    let _g = SLOT_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    // Sous verrou : `Engine::new` lit l'env à la construction —
    // fenêtre courte, non parallèle (même motif que c500).
    std::env::set_var("EXPLORER_NO_SLOTS", "1");
    let r = eval("function f(){let x=3,y=4;try{throw x*y}catch(e){let z=e+1;return z}}f()");
    // `x=1` avant `let x` aussi : remontée inchangée sans slots.
    let r2 = eval("function f(){var x=0;{x=1;let x=2;};return x}f()");
    std::env::remove_var("EXPLORER_NO_SLOTS");
    assert_eq!(r, "13");
    assert_eq!(r2, "1");
}
