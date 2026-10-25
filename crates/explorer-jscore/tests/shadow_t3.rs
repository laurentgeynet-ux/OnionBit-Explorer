//! T3 — slots/répartition flat dans le miroir ;
//! T5 — `<template shadowrootmode>` déclaratif.
use explorer_jscore::{Dom, Engine};
use std::cell::RefCell;
use std::rc::Rc;

fn eng(html: &str) -> Engine {
    let mut e = Engine::new();
    e.install_dom(&Rc::new(RefCell::new(Dom::from_html(html))));
    e
}

fn ev(e: &mut Engine, s: &str) -> String {
    match e.eval(s) {
        Ok(v) => e.describe(v),
        Err(x) => x.to_string().chars().take(160).collect(),
    }
}

#[test]
fn t3_slot_distribution() {
    // Hôte + shadow `{<slot name=a><slot>}` — distribution
    // flat : `slot=a` → named, les autres (éléments + texte)
    // → slot par défaut.
    let mut e = eng("<div id=h><i slot=a>x</i><b>y</b>text</div>");
    e.eval(
        "var h=document.getElementById('h');var r=h.attachShadow({mode:'open'});\
            var sl1=document.createElement('slot');sl1.name='a';r.appendChild(sl1);\
            var sl0=document.createElement('slot');r.appendChild(sl0);",
    )
    .unwrap();
    assert_eq!(
        ev(&mut e, "sl1.assignedElements().map(n=>n.tagName).join(',')"),
        "I"
    );
    assert_eq!(
        ev(&mut e, "sl0.assignedElements().map(n=>n.tagName).join(',')"),
        "B"
    );
    // `assignedNodes` inclut le texte sur le slot par défaut.
    assert_eq!(
        ev(
            &mut e,
            "sl0.assignedNodes().length+'/'+sl1.assignedNodes().length"
        ),
        "2/1"
    );
    // `el.assignedSlot` résout dans les deux sens.
    assert_eq!(
        ev(
            &mut e,
            "String(h.querySelector('i').assignedSlot===sl1)+'/'+String(h.querySelector('b').assignedSlot===sl0)"
        ),
        "true/true"
    );
    // `el.slot=` redistribue.
    assert_eq!(
        ev(
            &mut e,
            "h.querySelector('b').slot='a';sl1.assignedElements().map(n=>n.tagName).join(',')"
        ),
        "I,B"
    );
    // `slot.name` reflète l'attribut.
    assert_eq!(ev(&mut e, "sl1.name+'/'+sl0.name"), "a/");
}

#[test]
fn t5_shadowrootmode_declarative() {
    // `<template shadowrootmode=open>` devient la racine du
    // parent — `host`/`shadowRoot`/`assignedSlot` vivants.
    let mut e = eng(
        "<div id=h><template shadowrootmode=open><slot name=a></slot></template><i slot=a>x</i></div>",
    );
    assert_eq!(
        ev(
            &mut e,
            "var h=document.getElementById('h');var r=h.shadowRoot;\
            String(!!r)+'/'+String(r.host===h)+'/'+\
            String(h.querySelector('i').assignedSlot===r.querySelector('slot'))"
        ),
        "true/true/true"
    );
    // `closed` → `shadowRoot` null, racine réelle quand même.
    let mut e = eng("<div id=h><template shadowrootmode=closed><slot></slot></template></div>");
    assert_eq!(
        ev(
            &mut e,
            "var h=document.getElementById('h');String(h.shadowRoot===null)+'/'+\
            (function(){try{h.attachShadow({mode:'open'});return 'reattached'}catch(x){return 'thrown'}})()"
        ),
        "true/thrown"
    );
}
