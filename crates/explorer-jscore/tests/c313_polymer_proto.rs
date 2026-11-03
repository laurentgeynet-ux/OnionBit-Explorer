// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! §B-4(c) — sonde bornée du mur YouTube : `ytd-app` upgradée mais
//! `connectedCallback` jamais tiré (ctor meurt dans
//! `_initializeProperties`, `created()` jamais atteint — live :
//! `attached=false`, `__dataReady=false`, `hostElement of Undefined`
//! à `kevlar_base:40872`). Mécanisme suspecté : perte SÉLECTIVE de
//! méthodes lifecycle sur les protos de mixin profonds —
//! `Polymer.Element`/`y1` manque `created`/`connectedCallback`/
//! `_registered`/`_applyListeners`/`_takeAttributes` alors que
//! `attached`/`ready`/`_initializeProperties` survivent — le throw
//! `cannot read 'call' of Undefined` naît des chaînes
//! `Y.prototype.X.call(this)` non gardées.
//!
//! Fixture : `_scratch/kevlar_base.es5.js` (bundle réel 10,8 Mio,
//! gitignorée — SKIP silencieux si absente).

use explorer_jscore::{Dom, Engine};
use std::cell::RefCell;
use std::rc::Rc;

const KEVLAR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../_scratch/kevlar_base.es5.js");

fn ev(e: &mut Engine, src: &str) -> String {
    match e.eval(src) {
        Ok(v) => e.describe(v),
        Err(t) => format!("ERR:{t}").chars().take(220).collect(),
    }
}

/// Repro fidèle du terrain : le vrai bundle kevlar évalué
/// in-process, puis inspection des méthodes lifecycle sur
/// `Polymer.Element.prototype` — les `u` doivent être `f`.
#[test]
#[ignore = "sonde — fixture bundle réel non versionnée"]
fn c313_polymer_element_proto_methods() {
    let Ok(src) = std::fs::read_to_string(KEVLAR) else {
        eprintln!("SKIP {KEVLAR} absent");
        return;
    };
    let mut e = Engine::new();
    e.install_timers();
    // `<x-up>` présent au parse — l'upgrade au `define` reproduit
    // le vrai chemin `ce_upgrade_node` de `ytd-app` (élément déjà
    // attaché quand la classe arrive).
    e.install_dom(&Rc::new(RefCell::new(Dom::from_html(
        "<html><body><x-up></x-up><x-yt></x-yt><div id=x></div></body></html>",
    ))));
    // Le bundle jette probablement quelque part (APIs browser) —
    // les globals installés avant l'erreur restent inspectables.
    let _ = e.eval(&src);
    for name in [
        "Polymer",
        "Polymer.Element",
        "Polymer.Element.prototype.connectedCallback",
        "Polymer.Element.prototype.created",
        "Polymer.Element.prototype._registered",
        "Polymer.Element.prototype._initializeProperties",
        "Polymer.Element.prototype._applyListeners",
        "Polymer.Element.prototype._takeAttributes",
        "Polymer.Element.prototype._ensureAttributes",
        "Polymer.Element.prototype.attached",
        "Polymer.Element.prototype.ready",
        "Polymer.Element.prototype.disconnectedCallback",
        "Polymer.Element.prototype.attributeChangedCallback",
        "customElements.get('ytd-app')",
    ] {
        let r = ev(&mut e, &format!("typeof {name}"));
        println!("{name:>62} = {r}");
    }
    // Chaîne proto de la classe ytd-app si définie.
    let chain = ev(
        &mut e,
        "(function(){var C=customElements.get&&customElements.get('ytd-app');if(typeof C!=='function')return'no-ytd-app';var p=C.prototype,s=[];for(var d=0;p&&d<12;d++){var n=(p.constructor&&p.constructor.name)||'?';s.push(n+':'+(typeof p.created)[0]+(typeof p.connectedCallback)[0]+(typeof p._registered)[0]+(typeof p._initializeProperties)[0]+(typeof p._applyListeners)[0]+(typeof p._takeAttributes)[0]+(typeof p.attached)[0]+(typeof p.ready)[0]);p=Object.getPrototypeOf(p)}return s.join('|')})()",
    );
    println!("chain: {chain}");
    // La vraie machine Polymer in-process : construire une CE
    // via `Polymer.Class`/`mixinBehaviors` et mesurer les méthodes
    // lifecycle générées — c'est là que le drop sélectif doit
    // se montrer (terrain : `created`/`_registered`/`_applyListeners`
    ///`_takeAttributes`/`_ensureAttributes` absents).
    for name in [
        "Polymer.Class",
        "Polymer.mixinBehaviors",
        "Polymer.dedupingMixin",
        "Polymer.GenerateClassFromInfo",
        "Polymer.LegacyElementMixin",
        "Polymer.ElementMixin",
        "Polymer.PropertyEffects",
    ] {
        println!("{name:>42} = {}", ev(&mut e, &format!("typeof {name}")));
    }
    let r = ev(&mut e, "(function(){try{var q=Polymer.Class({is:'x-probe',behaviors:[]});return'Class ok|cr='+(typeof q.prototype.created)+'|cc='+(typeof q.prototype.connectedCallback)+'|reg='+(typeof q.prototype._registered)+'|aL='+(typeof q.prototype._applyListeners)+'|tA='+(typeof q.prototype._takeAttributes)+'|eA='+(typeof q.prototype._ensureAttributes)+'|at='+(typeof q.prototype.attached)+'|rd='+(typeof q.prototype.ready)}catch(x){return 'THREW:'+String(x.stack||x.message||x).slice(0,300)}})()");
    println!("Polymer.Class → {r}");
    let r2 = ev(&mut e, "(function(){try{var q=Polymer.mixinBehaviors([{created:function(){this.__b=1}}],Polymer.Element);customElements.define('x-probe2',q);var el=document.createElement('x-probe2');document.body.appendChild(el);return'def+attach ok|att='+el.attached+'|__b='+el.__b+'|ccT='+(typeof q.prototype.connectedCallback)+'|crT='+(typeof q.prototype.created)}catch(x){return 'THREW:'+String(x.stack||x.message||x).slice(0,300)}})()");
    println!("mixinBehaviors+define → {r2}");
    // Les couches réelles de ytd-app : LegacyElementMixin et
    // dedupingMixin appliqués sur Polymer.Element — la perte
    // sélective doit se montrer sur la chaîne résultante.
    let r3 = ev(&mut e, "(function(){try{var L=Polymer.LegacyElementMixin(Polymer.Element),s=[];for(var p=L.prototype,d=0;p&&d<8;d++){var n=(p.constructor&&p.constructor.name)||'?';s.push(n+':'+(typeof p.created)[0]+(typeof p.connectedCallback)[0]+(typeof p._registered)[0]+(typeof p._applyListeners)[0]+(typeof p._takeAttributes)[0]+(typeof p._ensureAttributes)[0]+(typeof p.attached)[0]+(typeof p.ready)[0]);p=Object.getPrototypeOf(p)}return s.join('|')}catch(x){return'THREW:'+String(x.message||x).slice(0,150)}})()");
    println!("LegacyElementMixin(Element) chain → {r3}");
    let r4 = ev(&mut e, "(function(){try{var M=Polymer.dedupingMixin(function(b){return class extends b{foo(){return 1} _bar(){return 2}}});var C=M(Polymer.Element);return'M ok|foo='+(typeof C.prototype.foo)+'|_bar='+(typeof C.prototype._bar)+'|cc='+(typeof C.prototype.connectedCallback)}catch(x){return'THREW:'+String(x.message||x).slice(0,150)}})()");
    println!("dedupingMixin → {r4}");
    let r5 = ev(&mut e, "(function(){try{var C=Polymer.Class({is:'x-p3',behaviors:[{created:function(){this.__z=1},attached:function(){this.__a=1}}]});customElements.define('x-p3',C);var el=document.createElement('x-p3');document.body.appendChild(el);return'ok|__z='+el.__z+'|__a='+el.__a+'|att='+el.attached+'|root='+(el.root===el)+'|dR='+el.__dataReady}catch(x){return'THREW:'+String(x.stack||x.message||x).slice(0,300)}})()");
    println!("Class+define+attach réel → {r5}");
    // Statiques ctor.__proto__ — `Y._initializeProperties` appelle
    // `this.constructor.finalize()`/`_finalizeTemplate` : si la
    // chaîne `__proto__` des constructeurs est cassée, `finalize`
    // résout undefined → le ctor meurt là.
    let r6 = ev(&mut e, "(function(){var C=Polymer.Class({is:'x-p4'});var s=[];var f=C;for(var d=0;f&&d<10;d++){s.push((f.name||'?')+':'+(typeof f.finalize)[0]+(typeof f._finalizeTemplate)[0]+(typeof f._importPath||'u')[0]);f=Object.getPrototypeOf(f)}return 'Cfin='+(typeof C.finalize)+'|Cft='+(typeof C._finalizeTemplate)+'|PEfin='+(typeof Polymer.Element.finalize)+'|PEft='+(typeof Polymer.Element._finalizeTemplate)+'|chain='+s.join('|')})()");
    println!("statics → {r6}");
    // Le vrai chemin `ce_upgrade_node` : `<x-up>` est un élément
    // parseur attaché — `customElements.define` doit upgrader
    // `el` en place (ctor + `connectedCallback` à l'attache).
    let r7 = ev(&mut e, "(function(){try{var C=Polymer.Class({is:'x-up',behaviors:[{_noAccessors:!0,actionRouterIsRoot:!1,created:function(){this.actionRouterNode=this},attached:function(){this.__att=1}}]});customElements.define('x-up',C);var el=document.querySelector('x-up');return'upgraded|ctor='+(el.constructor&&el.constructor.name)+'|root='+(el.root===el)+'|rn='+(typeof el.actionRouterNode)+'|att='+(typeof el.attached)+'|dR='+el.__dataReady+'|__data='+!!el.__data}catch(x){return'THREW:'+String(x.stack||x.message||x).slice(0,280)}})()");
    println!("upgrade parser element → {r7}");
    // La classe `ytd-app` réelle est bâtie par `GenerateClassFromInfo`
    // (`JZA`) avec le comportement actionRouter verbatim —
    // `attached` → `registerActionRouterEventListeners_` →
    // `this.actionRouterNode.hostElement` (kevlar_base:40872).
    let r8 = ev(&mut e, "(function(){try{var B={_noAccessors:!0,actionRouterIsRoot:!1,created:function(){this.actionRouter_=1;this.actionRouterNode=this.actionRouterIsRoot?document.body:this;this.onYtActionBoundListener_=function(){};this.actionRouterListenersRegistered_=!1},attached:function(){this.registerActionRouterEventListeners_()},registerActionRouterEventListeners_:function(){if(!this.actionRouterListenersRegistered_){this.actionRouterListenersRegistered_=!0;var n;((n=this.actionRouterNode.hostElement)!=null?n:this.actionRouterNode).addEventListener('yt-action',this.onYtActionBoundListener_)}}};var C=Polymer.Class({is:'x-yt',behaviors:[B]});customElements.define('x-yt',C);var el=document.querySelector('x-yt');return'ok|att='+el.attached+'|rn='+(typeof el.actionRouterNode)+'|dR='+el.__dataReady}catch(x){return'THREW:'+String(x.stack||x.message||x).slice(0,280)}})()");
    println!("ytd-app verbatim (GCFI+behavior) → {r8}");
    // `rn=object`+`__data` (created OK) mais `att`/`dR` jamais posés
    // — `connectedCallback` meurt aussi in-process : la stack
    // nomme le `.call` sur undefined.
    let r9 = ev(&mut e, "(function(){var el=document.querySelector('x-yt');try{el.connectedCallback();return'cc-ok|att='+el.attached+'|dR='+el.__dataReady}catch(x){return'THREW:'+String(x.stack||x.message||x).slice(0,320)}})()");
    println!("x-yt.connectedCallback() → {r9}");
    let r10 = ev(&mut e, "(function(){var el=document.querySelector('x-yt');try{el.ready();return'ready-ok|dR='+el.__dataReady}catch(x){return'THREW:'+String(x.stack||x.message||x).slice(0,320)}})()");
    println!("x-yt.ready() → {r10}");
    // Le set exact des méthodes own sur `Polymer.Element.prototype`
    // — element-mixin en définit ~20 ; la perte sélective doit se
    // lire ici (boundary du drop).
    let r11 = ev(&mut e, "(function(){var p=Polymer.Element.prototype,n=Object.getOwnPropertyNames(p);return 'own='+n.length+':'+n.sort().join(',')})()");
    println!("PE.prototype own props → {r11}");
    let r12 = ev(&mut e, "(function(){var p=Object.getPrototypeOf(Polymer.Element.prototype);return 'parent='+(p&&p.constructor&&p.constructor.name)+'|own='+(p?Object.getOwnPropertyNames(p).length:-1)+'|tA='+(p&&typeof p._takeAttributes)+'|eA='+(p&&typeof p._ensureAttributes)})()");
    println!("PE.prototype parent → {r12}");
}
