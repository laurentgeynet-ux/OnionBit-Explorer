// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Gate §B-119 — `window.location` unforgeable.
//!
//! Vol 884 vercel ×43 : chaque prefetch Next.js faisait
//! `new URL(addBasePath(href), window.location.href)` — après
//! un `location = x` (prop data writable remplacée par le
//! brut), `.href` rendait `undefined` → `Invalid URL:` par
//! lien de nav. Spec : `location` est un accesseur
//! non-configurable dont le setter **navigue**.

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
        eprintln!("[b119] FAIL {name} -> {r}");
        return false;
    }
    true
}

#[test]
fn b119_location_survit_aux_assignations() {
    let mut e = Engine::new();
    e.install_dom(&Rc::new(RefCell::new(Dom::from_html("<a href='/x'>y</a>"))));
    e.set_page_url("https://vercel.com/");
    let nav = Rc::new(RefCell::new(Vec::<String>::new()));
    let nl = nav.clone();
    e.set_nav_hooks(
        Rc::new(move |u: &str| nl.borrow_mut().push(u.to_string())),
        Rc::new(|_d: i32| {}),
        0,
    );

    let mut bad = false;
    // Verbatim du vol : prefetch `new URL(rel, location.href)`.
    bad |= !ok(
        &mut e,
        "prefetch",
        "new URL('/contact/sales', window.location.href).href === 'https://vercel.com/contact/sales'",
    );
    // `location.href = u` navigue (setter Location) — l'objet
    // continue de servir l'URL courante.
    bad |= !ok(
        &mut e,
        "href=",
        "location.href = '/p'; location.href === 'https://vercel.com/'",
    );
    // `location = u` navigue — la prop n'est **pas** remplacée
    // (unforgeable : accesseur non configurable).
    bad |= !ok(
        &mut e,
        "location=",
        "window.location = '/nav'; location.href === 'https://vercel.com/'",
    );
    bad |= !ok(&mut e, "identity", "location === window.location");
    // `delete` absorbé — prop non configurable.
    bad |= !ok(
        &mut e,
        "delete",
        "delete window.location; location.href === 'https://vercel.com/'",
    );
    bad |= !ok(
        &mut e,
        "descriptor",
        "Object.getOwnPropertyDescriptor(window,'location').configurable === false",
    );
    // Après les assignations : le prefetch verbatim repasse.
    bad |= !ok(
        &mut e,
        "prefetch-after",
        "new URL('/contact/sales', window.location.href).href === 'https://vercel.com/contact/sales'",
    );
    // Les deux assignations sont devenues des navigations.
    assert_eq!(
        *nav.borrow(),
        vec!["/p".to_string(), "/nav".to_string()],
        "les assignations `location(.href)=` doivent arriver au hook navigate"
    );
    assert!(!bad, "b119 gate — voir [b119] ci-dessus");
}
