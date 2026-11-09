// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! JS-C455 (JT-11) — verrou de la surface i18n
//! **énumérable** : `[[getSimpleString]]`/`[[countryCode]]`
//! du vol 701 sont des clés d'objets **service internes
//! `YouTube`** (produits par `yt.i18n` côté site — pas une
//! API web, le moteur ne peut pas les fournir honnêtement).
//! La surface navigateur i18n existe déjà ; ce test la
//! fige, le résiduel se nomme par sonde au prochain vol.

use explorer_jscore::{Dom, Engine};
use std::cell::RefCell;
use std::rc::Rc;

fn engine() -> Engine {
    let mut e = Engine::new();
    e.install_dom(&Rc::new(RefCell::new(Dom::from_html(
        "<html><body></body></html>",
    ))));
    e
}

fn ev(e: &mut Engine, src: &str) -> String {
    match e.eval(src) {
        Ok(v) => e.describe(v),
        Err(err) => panic!("{src} -> {err}"),
    }
}

/// La surface i18n navigateur est présente — rien
/// d'énumérable ne manque pour un site qui l'utiliserait.
#[test]
fn c455_i18n_enumerable_surface_present() {
    let mut e = engine();
    assert_eq!(
        ev(
            &mut e,
            "typeof Intl + '|' + typeof Intl.DateTimeFormat + '|' + \
             typeof Intl.NumberFormat + '|' + typeof Intl.Collator + '|' + \
             typeof Intl.getCanonicalLocales + '|' + \
             navigator.language + '|' + navigator.languages[0]"
        ),
        "object|function|function|function|function|en-US|en-US"
    );
    // `Intl.Segmenter` (borné documenté) + canonical.
    assert_eq!(
        ev(
            &mut e,
            "typeof Intl.Segmenter + '|' + Intl.getCanonicalLocales('fr-FR')[0]"
        ),
        "function|fr-FR"
    );
}

/// `Intl` sert les formatters réels — un site qui
/// construirait `countryCode`/`locale` côté données les
/// trouve (date/nombre/localisation).
#[test]
fn c455_i18n_formatters_real() {
    let mut e = engine();
    assert_eq!(
        ev(&mut e, "new Intl.NumberFormat('en').format(1234.5)"),
        "1,234.5"
    );
}
