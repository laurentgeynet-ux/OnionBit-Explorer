// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Gate §B-116 — batterie empirique AVANT tout fix.
//!
//! `Invalid scheme` n'existe pas dans notre code : le jeté est
//! interne au polyfill core-js (`parseURL` states `SCHEME`/
//! `SCHEME_START` — sous `stateOverride`, donc **setter**, ou
//! `NO_SCHEME` sans base). Le polyfill ne s'installe que si la
//! détection `USE_NATIVE_URL` échoue — cette suite reproduit
//! chaque primitive que la détection vérifie : un `false` =
//! un check qui fait installer le polyfill.
//!
//! Chaque test évalue un snippet et attend `true`.

use explorer_jscore::{Dom, Engine};
use std::cell::RefCell;
use std::rc::Rc;

fn eval(e: &mut Engine, src: &str) -> String {
    match e.eval(src) {
        Ok(v) => format!("{v:?}"),
        Err(err) => format!("ERR:{err:?}"),
    }
}

fn ok(e: &mut Engine, name: &str, src: &str) -> bool {
    let r = eval(e, src);
    if r != "Bool(true)" {
        eprintln!("[gate] FAIL {name} -> {r}");
        return false;
    }
    true
}

/// Batterie `USE_NATIVE_URL` — chaque primitive que la
/// détection core-js éprouve. Non-fatal : on rapporte tous
/// les échecs d'un coup.
#[test]
fn b116_native_url_detection_battery() {
    let mut e = Engine::new();
    let checks: &[(&str, &str)] = &[
        ("rel_base", "new URL('b?a=1&b=2&c=3','https://a').href === 'https://a/b?a=1&b=2&c=3'"),
        ("abs_input", "new URL('https://x/y','https://a').href === 'https://x/y'"),
        ("pathname_enc", "(function(){var u=new URL('b','https://a');u.pathname='c d';return u.href==='https://a/c%20d'})()"),
        ("sp_sort", "typeof new URL('b','https://a').searchParams.sort === 'function'"),
        ("sp_get", "new URL('b?c=3','https://a').searchParams.get('c') === '3'"),
        ("usp_str", "String(new URLSearchParams('?a=1')) === 'a=1'"),
        ("usp_getall", "new URLSearchParams('a=1&a=2').getAll('a').join() === '1,2'"),
        ("username", "new URL('https://a@b').username === 'a'"),
        ("hash", "new URL('https://a#b').hash === '#b'"),
        ("port", "new URL('https://a:81/x').port === '81'"),
        ("usp_ctor_usp", "new URLSearchParams(new URLSearchParams('a=b')).has('a')"),
        ("usp_has_val", "var s=new URLSearchParams('a=1&a=2');s.has('a','2') && !s.has('a','3')"),
        ("usp_delete_val", "var s=new URLSearchParams('a=1&a=2');s['delete']('a','2');s.get('a')==='1'"),
        ("usp_undef_val", "var s=new URLSearchParams('a=1');s['delete']('a',undefined);s.get('a')==='1'"),
        ("canParse", "typeof URL.canParse === 'function' && URL.canParse('https://a') && !URL.canParse('')"),
        ("url_parse_static", "typeof URL.parse === 'function' && URL.parse('https://a').href === 'https://a/' && URL.parse('x') === null"),
        ("usp_size", "new URLSearchParams('a=1&b=2').size === 2"),
        ("foreach_live", "var s=new URLSearchParams('a=1&b=2&c=3');var seen=[];s.forEach(function(v,k){seen.push(k);s['delete']('b')});seen.join(',')==='a,c'"),
        // Itérateur vivant via `next()` explicite — `for..of`
        // reste EAGER côté moteur (`iter_values` draine tout
        // avant le corps — divergence nommée, hors §B-116).
        ("iter_live", "var s=new URLSearchParams('a=1&b=2');var it=s.entries();it.next();s.append('d','4');var n=0;while(true){var r=it.next();if(r.done)break;n++}n===2"),
        ("idna", "new URL('http://тест').host === 'xn--e1aybc'"),
        ("scheme_rel_fallback", "(function(){try{new URL('x');return false}catch(e){return true}})()"),
        ("empty_throws", "(function(){try{new URL('');return false}catch(e){return true}})()"),
        ("undef_base_throws", "(function(){try{new URL('x',undefined);return false}catch(e){return true}})()"),
        ("tab_nl_strip", "new URL('https:\\t//exa\\nmple.com/\\rx').host === 'example.com'"),
    ];
    let mut fails = Vec::new();
    for (name, src) in checks {
        if !ok(&mut e, name, src) {
            fails.push(*name);
        }
    }
    assert!(fails.is_empty(), "gate fails: {fails:?}");
}

/// Sondes regex — le `SCHEME`/`SCHEME_START` du polyfill
/// vit dans des classes de caractères : si `/i` ou `\d`
/// divergent, TOUTE URL est rejetée.
#[test]
fn b116_scheme_regex_probes() {
    let mut e = Engine::new();
    let checks: &[(&str, &str)] = &[
        ("alpha_i", "/[a-z]/i.exec('H') !== null"),
        (
            "scheme_class",
            "/^[a-zA-Z][a-zA-Z\\d+\\-.]*$/.test('web+foo')",
        ),
        ("scheme_class_dash", "/[\\d+\\-.a-z]/i.exec('-') !== null"),
        ("scheme_class_dot", "/[\\d+\\-.a-z]/i.exec('.') !== null"),
        (
            "scheme_class_colon_neg",
            "/[\\d+\\-.a-z]/i.exec(':') === null",
        ),
        ("digit_class", "/\\d/.exec('9') !== null"),
    ];
    let mut fails = Vec::new();
    for (name, src) in checks {
        if !ok(&mut e, name, src) {
            fails.push(*name);
        }
    }
    assert!(fails.is_empty(), "regex fails: {fails:?}");
}

/// `Object.defineProperty` getter (`DESCRIPTORS` du polyfill) +
/// `location.*` sous ctx seedé — `location.protocol === ""` sur un
/// ctx non seedé nourrirait `x.protocol = ''` → `Invalid scheme`
/// côté polyfill.
#[test]
fn b116_descriptors_et_location() {
    let mut e = Engine::new();
    e.install_dom(&Rc::new(RefCell::new(Dom::from_html(
        "<html><body></body></html>",
    ))));
    let checks: &[(&str, &str)] = &[
        ("defprop_get", "Object.defineProperty({},'a',{get:function(){return 7}}).a === 7"),
        ("defprop_set", "var o=Object.defineProperty({},'a',{set:function(v){this._a=v},get:function(){return this._a}});o.a=9;o.a===9"),
        ("location_typeof", "typeof location === 'object'"),
        ("location_href_str", "typeof location.href === 'string'"),
        ("location_proto_str", "typeof location.protocol === 'string'"),
        ("location_tostring", "typeof location.toString === 'function'"),
    ];
    let mut fails = Vec::new();
    for (name, src) in checks {
        if !ok(&mut e, name, src) {
            fails.push(*name);
        }
    }
    // `hooks.page_url` non seedé dans un ctx de test pur :
    // on DOCUMENTE la valeur (suspect, pas un verdict).
    eprintln!("[gate] location.href = {}", eval(&mut e, "location.href"));
    eprintln!(
        "[gate] location.protocol = {}",
        eval(&mut e, "location.protocol")
    );
    assert!(fails.is_empty(), "fails: {fails:?}");
}
