// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! §B-208 — `ReferenceError: _0x16a433 is not defined` (fr24, vol 958,
//! `__modulepreload__-C46svxnw.js`) : le boilerplate auto-défense de
//! javascript-obfuscator fait `function r4(){const e=[…];
//! return r4=function(){return e},r4()}` — la déclaration se réassigne à
//! une fermeture qui retourne le tableau de chaînes capturé. Atteindre la
//! **position** de la déclaration ré-instanciait et re-liait `r4` à une
//! fonction fraîche : `zn` relisait alors un tableau non tourné → clés
//! décodées fausses → la branche piège `a!==b` tirait sur l'identifiant
//! empoisonné jamais déclaré.
//! Spec : une `FunctionDeclaration` n'exécute rien à sa position ; la
//! liaison conserve la valeur réassignée. Verrous : réassignation
//! préservée (identité + contenu), déclaration sœur de même nom en
//! boucle (lit différent → nouvelle closure conservée), liaison
//! `undefined` installée.

use explorer_jscore::Engine;

fn ev(js: &str) -> String {
    let mut e = Engine::new();
    match e.eval(js) {
        Ok(v) => e.describe(v),
        Err(err) => panic!("{js} -> {err}"),
    }
}

#[test]
fn decl_position_ne_clobbe_pas_la_reassignation() {
    // Verbatim du pattern obfuscator : la déclaration se réassigne à une
    // fermeture capturant `e`. Atteindre sa position doit être un no-op.
    assert_eq!(
        ev("var e1=r4(); e1.push('A'); function r4(){const e=[];return r4=function(){return e},r4();}\
            var e2=r4(); (e1===e2)+'|'+e2.length+'|'+e2[0]"),
        "true|1|A"
    );
    // La fermeture réassignée survit aussi pour une fonction nommée
    // différente du nom déclaré.
    assert_eq!(
        ev("var a=g(); function g(){return g=function z(){return 'new'},1;} a+'|'+g()"),
        "1|new"
    );
}

#[test]
fn decl_soeur_meme_nom_garde_sa_nouvelle_closure() {
    // Itération : une liaison qui tient une fonction dont le lit porte
    // le MÊME nom mais n'est pas cette déclaration reçoit une nouvelle
    // closure (comportement historique conservé).
    assert_eq!(
        ev(
            "var r=[]; for (var i=0;i<2;i++){ if(i===0){function f(){return 'x'} r.push(f())}\
            else {function f(){return 'y'} r.push(f())} } r.join(',')"
        ),
        "x,y"
    );
    // La liaison `undefined` reçoit toujours l'instance (décl de bloc).
    assert_eq!(
        ev("var h; { h = 42; function q(){return 7} } typeof q + '|' + h"),
        "function|42"
    );
}
