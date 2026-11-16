// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! JS-C561 — second lot Pareto BCD : protos d'interfaces
//! peuplés (`in`/`instanceof` exacts), impls bornées
//! (`Highlight` Set-backed, `DataTransfer` store réel,
//! `DOMQuad`, `VTTRegion`, `FileReaderSync`, tables).

use explorer_jscore::{Dom, Engine};
use std::cell::RefCell;
use std::rc::Rc;

const PAGE: &str = r#"<html><body>
<table id="t"><tbody><tr><td>a</td></tr></tbody></table>
<input id="i" type="number" value="10" step="2">
<div id="d"></div>
</body></html>"#;

fn engine() -> Engine {
    let mut e = Engine::new();
    e.install_dom(&Rc::new(RefCell::new(Dom::from_html(PAGE))));
    e
}

fn ev(e: &mut Engine, src: &str) -> String {
    match e.eval(src) {
        Ok(v) => e.describe(v),
        Err(err) => panic!("{src} -> {err}"),
    }
}

/// Protos d'interfaces : la surface IDL est `in`-visible
/// (`'x' in Ctor.prototype`) et les instances chaînées sont
/// `instanceof`-exactes.
#[test]
fn interface_protos() {
    let mut e = engine();
    assert_eq!(ev(&mut e, "'height' in Screen.prototype"), "true");
    assert_eq!(ev(&mut e, "screen instanceof Screen"), "true");
    assert_eq!(ev(&mut e, "'lock' in ScreenOrientation.prototype"), "true");
    assert_eq!(
        ev(&mut e, "screen.orientation instanceof ScreenOrientation"),
        "true"
    );
    assert_eq!(ev(&mut e, "'charging' in BatteryManager.prototype"), "true");
    assert_eq!(
        ev(&mut e, "'open' in StorageBucketManager.prototype"),
        "true"
    );
    assert_eq!(ev(&mut e, "'persist' in StorageBucket.prototype"), "true");
    assert_eq!(ev(&mut e, "'scale' in VisualViewport.prototype"), "true");
    assert_eq!(
        ev(&mut e, "visualViewport instanceof VisualViewport"),
        "true"
    );
    assert_eq!(ev(&mut e, "'target' in MutationRecord.prototype"), "true");
    assert_eq!(ev(&mut e, "'key' in KeyboardEvent.prototype"), "true");
    assert_eq!(
        ev(&mut e, "typeof KeyboardEvent.prototype.getModifierState"),
        "function"
    );
    assert_eq!(ev(&mut e, "'clientX' in Touch.prototype"), "true");
}

/// `Highlight`/`HighlightRegistry`/`EventCounts` — setlike/
/// maplike réels par chaînage `Set`/`Map`.
#[test]
fn highlight_and_registry() {
    let mut e = engine();
    assert_eq!(ev(&mut e, "typeof Highlight"), "function");
    assert_eq!(ev(&mut e, "typeof HighlightRegistry"), "function");
    assert_eq!(ev(&mut e, "typeof EventCounts"), "function");
    assert_eq!(
        ev(
            &mut e,
            "var h=new Highlight(1,2); h instanceof Highlight && h.size===2 && h.has(1)"
        ),
        "true"
    );
    assert_eq!(ev(&mut e, "'set' in HighlightRegistry.prototype"), "true");
    assert_eq!(
        ev(&mut e, "CSS.highlights instanceof HighlightRegistry"),
        "true"
    );
    assert_eq!(
        ev(
            &mut e,
            "CSS.highlights.set('x',h); CSS.highlights.get('x')===h && CSS.highlights.size===1"
        ),
        "true"
    );
    assert_eq!(
        ev(
            &mut e,
            "typeof performance.eventCounts==='object' && performance.eventCounts instanceof EventCounts"
        ),
        "true"
    );
}

/// `DOMPointReadOnly`/`DOMQuad`/`VTTRegion` — ctors réels.
#[test]
fn geometry_and_vtt() {
    let mut e = engine();
    assert_eq!(
        ev(
            &mut e,
            "var p=new DOMPointReadOnly(1,2); p instanceof DOMPointReadOnly && p.x===1 && typeof p.toJSON==='function'"
        ),
        "true"
    );
    assert_eq!(ev(&mut e, "typeof DOMPointReadOnly.fromPoint"), "function");
    assert_eq!(
        ev(
            &mut e,
            "var q=new DOMQuad({x:0,y:0},{x:10,y:0},{x:10,y:5},{x:0,y:5}); q instanceof DOMQuad && q.getBounds().width===10 && q.getBounds().height===5"
        ),
        "true"
    );
    assert_eq!(
        ev(&mut e, "typeof DOMQuad.fromRect==='function' && DOMQuad.fromRect({x:1,y:1,width:2,height:2}).p3.x===3"),
        "true"
    );
    assert_eq!(
        ev(
            &mut e,
            "var r=new VTTRegion(); r instanceof VTTRegion && r.lines===3 && r.width===100"
        ),
        "true"
    );
}

/// `DataTransfer` — store de formats réel (`setData`/`getData`/
/// `types`/`items`/`clearData`).
#[test]
fn data_transfer() {
    let mut e = engine();
    assert_eq!(ev(&mut e, "typeof DataTransfer"), "function");
    assert_eq!(
        ev(
            &mut e,
            "var dt=new DataTransfer(); dt.setData('text/plain','hello'); dt.getData('text/plain')"
        ),
        "hello"
    );
    assert_eq!(
        ev(
            &mut e,
            "dt.types.length===1 && dt.types[0]==='text/plain' && dt.items.length===1 && dt.items[0].kind==='string'"
        ),
        "true"
    );
    assert_eq!(
        ev(
            &mut e,
            "dt.clearData(); dt.types.length===0 && dt.items.length===0"
        ),
        "true"
    );
    assert_eq!(
        ev(&mut e, "'add' in DataTransferItemList.prototype"),
        "true"
    );
    assert_eq!(ev(&mut e, "'kind' in DataTransferItem.prototype"), "true");
}

/// `FileReaderSync`/`FileReader` protos — lectures réelles.
#[test]
fn file_readers() {
    let mut e = engine();
    assert_eq!(ev(&mut e, "typeof FileReaderSync"), "function");
    assert_eq!(
        ev(
            &mut e,
            "var rs=new FileReaderSync(); rs.readAsText(new Blob(['abc']))"
        ),
        "abc"
    );
    assert_eq!(ev(&mut e, "'readAsText' in FileReader.prototype"), "true");
    assert_eq!(ev(&mut e, "'onload' in FileReader.prototype"), "true");
    assert_eq!(ev(&mut e, "'readyState' in FileReader.prototype"), "true");
}

/// `Element`/`<input>`/`<table>` — membres réels du lot.
#[test]
fn element_input_table() {
    let mut e = engine();
    assert_eq!(
        ev(&mut e, "'requestPointerLock' in Element.prototype"),
        "true"
    );
    assert_eq!(
        ev(&mut e, "'startViewTransition' in Element.prototype"),
        "true"
    );
    assert_eq!(
        ev(
            &mut e,
            "var i=document.getElementById('i'); i.stepUp(2); i.value"
        ),
        "14"
    );
    assert_eq!(ev(&mut e, "i.stepDown(); i.value"), "12");
    assert_eq!(
        ev(&mut e, "'insertRow' in HTMLTableElement.prototype"),
        "true"
    );
    assert_eq!(
        ev(
            &mut e,
            "var t=document.getElementById('t'); var tr=t.insertRow(); tr.tagName"
        ),
        "TR"
    );
    assert_eq!(
        ev(
            &mut e,
            "t.rows.length===2 && (t.deleteRow(-1), t.rows.length===1)"
        ),
        "true"
    );
    assert_eq!(
        ev(
            &mut e,
            "var th=t.createTHead(); th.tagName==='THEAD' && t.createTHead()===th"
        ),
        "true"
    );
}
