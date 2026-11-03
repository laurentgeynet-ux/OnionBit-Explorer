// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! §B-4(c) — sonde bornée du mur `YouTube` : `ytd-app` upgradée mais
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

use explorer_jscore::{Dom, Engine, XhrRequest, XhrTransport};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

/// Transport XHR stub — enregistre sans jamais livrer : laisse
/// le bundle passer le feature-detect `XMLHttpRequest` (le vrai
/// mur §B-4(c) est en aval de cette API).
struct NullXhr {
    next: Cell<u64>,
}
impl XhrTransport for NullXhr {
    fn send(&self, _req: &XhrRequest) -> u64 {
        self.next.set(self.next.get() + 1);
        self.next.get()
    }
}

const KEVLAR: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../_scratch/kevlar_base.es5.js"
);

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
#[allow(clippy::too_many_lines)]
fn c313_polymer_element_proto_methods() {
    let Ok(src) = std::fs::read_to_string(KEVLAR) else {
        eprintln!("SKIP {KEVLAR} absent");
        return;
    };
    let mut e = Engine::new();
    e.install_timers();
    e.install_xhr(Rc::new(NullXhr { next: Cell::new(0) }));
    e.install_fetch();
    // `<x-up>`/`ytd-app` présents au parse — l'upgrade au `define`
    // reproduit le vrai chemin `ce_upgrade_node` de `ytd-app`
    // (élément déjà attaché quand la classe arrive ; sans lui le
    // bundle meurt sur `if(!n)throw Error("gl")` à 27532:2).
    e.install_dom(&Rc::new(RefCell::new(Dom::from_html(
        "<html><body><ytd-app></ytd-app><x-up></x-up><x-yt></x-yt><div id=x></div></body></html>",
    ))));
    // Le bundle jette probablement quelque part (APIs browser) —
    // les globals installés avant l'erreur restent inspectables.
    let r = e.eval(&src);
    if let Err(t) = r {
        println!("bundle eval THREW: {t}");
    }
    // Sonde JS-C313 — le drop à mi-composition : chaque rejet
    // `defineProperty`/`defineProperties`/`assign` pendant l'éval
    // du bundle nomme cible, clé, prop courante et raison exacte.
    let log = e.defprop_log_drain();
    println!("defprop_log : {} rejets", log.len());
    for l in &log {
        println!("  {l}");
    }
    let nlog = e.nullread_log_drain();
    println!("nullread_log : {} lectures nulles", nlog.len());
    for l in &nlog {
        println!("  {l}");
    }
    // État de l'élément `<ytd-app>` réel upgradé pendant l'éval :
    // terrain = ctor mort dans `_initializeProperties` (root jamais
    // posé, `__dataReady=false`, `attached` non appelé).
    println!(
        "ytd-app element → {}",
        ev(
            &mut e,
            "(function(){var n=document.querySelector('ytd-app');if(!n)return'no-element';return'ctor='+(n.constructor&&n.constructor.name)+'|cr='+(typeof n.created)+'|cc='+(typeof n.connectedCallback)+'|att='+(typeof n.attached)+'|rd='+(typeof n.ready)+'|root='+(n.root===n)+'|dR='+n.__dataReady+'|data='+(typeof n.__data)+'|tagName='+n.tagName})()",
        )
    );
    // Repro synthétique de la chaîne nommée : prop plain
    // `isSearchPage` (ACC `__data` via `createProperties`) + prop
    // `readOnly` `aFG` dont un getter BRUT (`Object.defineProperties`)
    // lit `this.isSearchPage` — `_definePropertyAccessor` capture
    // `this[aFG]` → getter → `proto.__data` undefined → throw si
    // `isSearchPage` est déjà ACC. En Chrome la capture doit tomber
    // sur la valeur brute (iS pas encore ACC / garde `in P`).
    println!(
        "synth aFG/iS → {}",
        ev(
            &mut e,
            "(function(){try{var C=Polymer.Class({is:'x-afg',properties:{isSearchPage:{type:Boolean,value:!1},applyFrostedGlassToChipBar:{type:Boolean,readOnly:!0}}});Object.defineProperties(C.prototype,{applyFrostedGlassToChipBar:{configurable:!0,enumerable:!0,get:function(){var n=this.isSearchPage;return!this.isHomePage||n}}});customElements.define('x-afg',C);var el=document.createElement('x-afg');document.body.appendChild(el);return'ok|root='+(el.root===el)+'|dR='+el.__dataReady+'|iS='+el.isSearchPage}catch(x){return'THREW:'+String(x.stack||x.message||x).slice(0,300)}})()",
        )
    );
    // Registres internes du proto `q` : `__dataProto.aFG` absent =
    // la capture `_definePropertyAccessor` a jeté à `this[aFG]`
    // (avant `__dataProto[h]=q`) ; `__dataHasAccessor` montre
    // quelles ACC sont installées au moment de la mort.
    println!(
        "proto regs → {}",
        ev(
            &mut e,
            "(function(){var C=customElements.get('ytd-app');if(typeof C!=='function')return'no-ytd';var p=C.prototype;var dp=Object.keys(p.__dataProto||{}),da=Object.keys(p.__dataHasAccessor||{});return'dP.n='+dp.length+'|dP.aFG='+dp.indexOf('applyFrostedGlassToChipBar')+'|dP.iS='+dp.indexOf('isSearchPage')+'|dHA.n='+da.length+'|dHA.aFG='+da.indexOf('applyFrostedGlassToChipBar')+'|dHA.iS='+da.indexOf('isSearchPage')+'|fin='+C.hasOwnProperty('__finalized')})()",
        )
    );
    // Descripteurs `isSearchPage`/`applyFrostedGlassToChipBar` par
    // couche de la chaîne `ytd-app` : en Chrome `proto.isSearchPage`
    // doit être une VALEUR (pas l'accessor `__data`) au moment où
    // `createProperties` lit `this[applyFrostedGlassToChipBar]` —
    // sinon le getter jette `__data` undefined comme chez nous.
    println!(
        "descriptors → {}",
        ev(
            &mut e,
            "(function(){var C=customElements.get('ytd-app');if(typeof C!=='function')return'no-ytd';var p=C.prototype,s=[];for(var d=0;p&&d<12;d++){var n=(p.constructor&&p.constructor.name)||'?';var a=Object.getOwnPropertyDescriptor(p,'isSearchPage'),b=Object.getOwnPropertyDescriptor(p,'applyFrostedGlassToChipBar');s.push(n+':iS='+(a?(a.get?'ACC':'val:'+typeof a.value):'-')+'|aFG='+(b?(b.get?'ACC':'val:'+typeof b.value):'-')+'|dd='+(p.hasOwnProperty('__data')?'has':'-'));p=Object.getPrototypeOf(p)}return s.join(' || ')})()",
        )
    );
    // Ordre des clés de `properties` : si `applyFrostedGlassToChipBar`
    // précède `isSearchPage` dans la map, Chrome capture sa valeur
    // AVANT que `isSearchPage` soit un accessor `__data` — aucun
    // throw. Une inversion (ou un `__data` proto manquant) fait
    // mourir `_initializeProperties` au getter `__data[h]`.
    println!(
        "props order → {}",
        ev(
            &mut e,
            "(function(){var C=customElements.get('ytd-app');if(typeof C!=='function')return'no-ytd';var p=C._properties||(C.prototype&&C.prototype._properties)||{};var ks=Object.keys(p);var d1=Object.getOwnPropertyDescriptor(C.prototype,'isSearchPage'),d2=Object.getOwnPropertyDescriptor(C.prototype,'applyFrostedGlassToChipBar');return'n='+ks.length+'|aFG@'+ks.indexOf('applyFrostedGlassToChipBar')+'|iS@'+ks.indexOf('isSearchPage')+'|iSBH@'+ks.indexOf('isSearchboxHidden')+'|iSget='+(d1&&d1.get?String(d1.get).slice(0,80):'-')+'|aFGget='+(d2&&d2.get?String(d2.get).slice(0,80):'-')})()",
        )
    );
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
    // `_takeAttributes`/`_ensureAttributes` absents).
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
    // Chaîne complète de `Polymer.Element.prototype` — chaque couche
    // avec son set own exact : nomme QUELLE couche (element-mixin,
    // PropertyEffects ~20 attendus, legacy ~22 attendus) est
    // absente ou tronquée — l'erreur d'éval est capturée aussi.
    let r13 = ev(&mut e, "(function(){var s=[],p=Polymer.Element.prototype,d=0;for(;p&&d<14;d++){var n=(p.constructor&&p.constructor.name)||'?';s.push(n+'('+Object.getOwnPropertyNames(p).length+'):'+Object.getOwnPropertyNames(p).sort().join(','));p=Object.getPrototypeOf(p)}return s.join(' || ')})()");
    println!("PE chain full → {r13}");
}
