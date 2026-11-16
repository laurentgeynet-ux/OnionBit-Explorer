// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! JS-C556 — `AbstractRange`/`StaticRange` IDL + comparaisons
//! `Range` (jalon N3.1 du plan vitrine) : chaîne proto spec,
//! `compareBoundaryPoints`/`comparePoint`/`isPointInRange`/
//! `intersectsNode`/`surroundContents`, `Selection.direction`/
//! `setPosition`/`getComposedRanges`/`modify` borné.

use explorer_jscore::{Dom, Engine};
use std::cell::RefCell;
use std::rc::Rc;

const PAGE: &str = r#"<html><body><div id="a"><i id="i1">un</i><b id="b">deux</b><u id="u">trois</u></div><div id="z">fin</div></body></html>"#;

fn engine() -> (Engine, Rc<RefCell<Dom>>) {
    let mut e = Engine::new();
    let dom = Rc::new(RefCell::new(Dom::from_html(PAGE)));
    e.install_dom(&dom);
    (e, dom)
}

fn ev(e: &mut Engine, src: &str) -> String {
    match e.eval(src) {
        Ok(v) => e.describe(v),
        Err(err) => panic!("{src} -> {err}"),
    }
}

/// Chaîne `instanceof` spec + ctor `AbstractRange` illégal.
#[test]
fn abstract_range_proto_chain() {
    let (mut e, _dom) = engine();
    assert_eq!(
        ev(
            &mut e,
            "var r = document.createRange();\
             (r instanceof Range) + '|' + (r instanceof AbstractRange) + '|' +\
             (Range.prototype.__proto__ === AbstractRange.prototype) + '|' +\
             (typeof AbstractRange) + '|' + ('startContainer' in AbstractRange.prototype) + '|' +\
             (Range.START_TO_START + '/' + Range.START_TO_END + '/' + Range.END_TO_END + '/' + Range.END_TO_START)"
        ),
        "true|true|true|function|true|0/1/2/3"
    );
    // Ctor non constructible (spec) → TypeError « Illegal constructor ».
    assert_eq!(
        ev(
            &mut e,
            "var caught; try { new AbstractRange(); caught = 'pas-err' } catch (x) { caught = (x instanceof TypeError) + '|' + /illegal/i.test(x.message) } caught"
        ),
        "true|true"
    );
    // L'accesseur proto vérifie la marque : appel sur objet
    // quelconque → « Illegal invocation » ; sur un range → la borne.
    assert_eq!(
        ev(
            &mut e,
            "var g = Object.getOwnPropertyDescriptor(AbstractRange.prototype, 'collapsed').get;\
             var r2 = document.createRange();\
             var a; try { g.call({}); a = 'pas-err' } catch (x) { a = 'Illegal:' + (x instanceof TypeError) }\
             a + '|' + g.call(r2)"
        ),
        "Illegal:true|true"
    );
}

/// `StaticRange` — ctor dict spec + bornes gelées.
#[test]
fn static_range_frozen() {
    let (mut e, _dom) = engine();
    assert_eq!(
        ev(
            &mut e,
            "var b = document.getElementById('b');\
             var s = new StaticRange({startContainer:b, startOffset:0, endContainer:b, endOffset:1});\
             (s instanceof StaticRange) + '|' + (s instanceof AbstractRange) + '|' +\
             (s.startContainer === b) + '|' + s.collapsed + '|' + (typeof s.setStart)"
        ),
        "true|true|true|false|undefined"
    );
    // Spec : `init` absent/invalide → TypeError.
    assert_eq!(
        ev(
            &mut e,
            "var c1; try { new StaticRange(); c1 = 'pas-err' } catch (x) { c1 = x instanceof TypeError }\
             c1 + '|' +\
             (function(){ try { new StaticRange({}); return 'pas-err' } catch (x) { return x instanceof TypeError } })()"
        ),
        "true|true"
    );
}

/// `Range.compare*`/`isPointInRange`/`intersectsNode`.
#[test]
fn range_point_comparisons() {
    let (mut e, _dom) = engine();
    // Range couvrant <b> : setStartBefore(b) / setEndAfter(b).
    assert_eq!(
        ev(
            &mut e,
            "var b = document.getElementById('b'), a = document.getElementById('a');\
             var r = document.createRange();\
             r.setStartBefore(b); r.setEndAfter(b);\
             var r2 = document.createRange(); r2.selectNode(b);\
             r.compareBoundaryPoints(0, r2) + '|' + r.compareBoundaryPoints(2, r2) + '|' +\
             r.comparePoint(b, 0) + '|' + r.isPointInRange(b, 0) + '|' +\
             r.intersectsNode(b) + '|' + r.intersectsNode(document.getElementById('z'))"
        ),
        "0|0|0|true|true|false"
    );
    // `how` invalide → NotSupportedError ; arg non-range → NotSupportedError.
    assert_eq!(
        ev(
            &mut e,
            "var r3 = document.createRange();\
             var x1; try { r3.compareBoundaryPoints(9, r3); x1='pas-err' } catch (x) { x1 = x.name }\
             var x2; try { r3.compareBoundaryPoints(0, {}); x2='pas-err' } catch (x) { x2 = x.name }\
             x1 + '|' + x2"
        ),
        "NotSupportedError|NotSupportedError"
    );
}

/// `surroundContents` déplace le contenu couvert dans le parent.
#[test]
fn range_surround_contents() {
    let (mut e, _dom) = engine();
    assert_eq!(
        ev(
            &mut e,
            "var a = document.getElementById('a'), b = document.getElementById('b');\
             var r = document.createRange();\
             r.setStartBefore(b); r.setEndAfter(b);\
             var em = document.createElement('em');\
             r.surroundContents(em);\
             (em.firstChild && em.firstChild.tagName) + '|' + (em.parentNode === a) + '|' +\
             (a.childNodes.length) + '|' + (r.startContainer === a) + '|' + r.endOffset"
        ),
        "B|true|3|true|2"
    );
}

/// `Selection` — `direction`, `setPosition`, `getComposedRanges`,
/// `modify` borné caractère.
#[test]
fn selection_new_surface() {
    let (mut e, _dom) = engine();
    assert_eq!(
        ev(
            &mut e,
            "var s = window.getSelection();\
             var i1 = document.getElementById('i1');\
             s.selectAllChildren(i1);\
             s.direction + '|' + ('deleteFromDocument' in s) + '|' +\
             s.getComposedRanges().length + '|' + (s.getComposedRanges()[0].collapsed)"
        ),
        "forward|true|1|false"
    );
    assert_eq!(
        ev(
            &mut e,
            "var s2 = window.getSelection();\
             var z = document.getElementById('z');\
             s2.setPosition(z.firstChild, 1);\
             s2.isCollapsed + '|' + s2.focusOffset + '|' + (s2.focusNode === z.firstChild) + '|' + s2.direction"
        ),
        "true|1|true|none"
    );
    // `modify('extend','forward','character')` — focus +1 car.
    assert_eq!(
        ev(
            &mut e,
            "var s3 = window.getSelection();\
             var t = document.getElementById('u').firstChild;\
             s3.setPosition(t, 1);\
             s3.modify('extend', 'forward', 'character');\
             s3.focusOffset + '|' + s3.isCollapsed"
        ),
        "2|false"
    );
}
