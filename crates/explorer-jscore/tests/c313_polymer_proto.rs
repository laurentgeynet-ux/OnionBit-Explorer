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
    // Séquence complète `__dataHasAccessor` = ordre d'install des
    // ACC sur le proto `q` : iS@21 vs aFG@34 — les voisins de iS
    // indiquent si elle vient du même pass `createProperties` que
    // aFG (props adjacentes) ou d'un mécanisme séparé.
    println!(
        "dHA order → {}",
        ev(
            &mut e,
            "(function(){var C=customElements.get('ytd-app');if(typeof C!=='function')return'no-ytd';var da=Object.keys(C.prototype.__dataHasAccessor||{});var i=da.indexOf('isSearchPage'),a=da.indexOf('applyFrostedGlassToChipBar');var lo=Math.max(0,i-2),hi=Math.min(da.length,i+3);var w=a-3<0?0:a-3,x=Math.min(da.length,a+2);return'iS ctx['+i+']='+da.slice(lo,hi).join(',')+' || aFG ctx['+a+']='+da.slice(w,x).join(',')+' || head='+da.slice(0,5).join(',')})()",
        )
    );
    // Les maps candidates qui alimentent `createProperties` :
    // `C._properties`/`C.properties`/`qd` decorator-metadata —
    // positions iS vs aFG dans l'ordre d'itération effectif.
    println!(
        "props maps → {}",
        ev(
            &mut e,
            "(function(){var C=customElements.get('ytd-app');if(typeof C!=='function')return'no-ytd';var s=[];['properties','_properties','__properties','__ownProperties'].forEach(function(k){var m=C[k]||C.prototype[k];if(m&&typeof m==='object'){var ks=Object.keys(m);s.push(k+'['+ks.length+']:iS@'+ks.indexOf('isSearchPage')+'/aFG@'+ks.indexOf('applyFrostedGlassToChipBar'))}else s.push(k+':-')});var c=C.prototype.constructor;s.push('ctor.prop='+(c&&c!==C?Object.keys(c._properties||{}).length:'same'));return s.join(' | ')})()",
        )
    );
    // La clé : `aRc` (le décorateur `_.YG`) fait `P.get=void 0` puis
    // `_.V` re-définit la prop — spec : `get` PRÉSENT à `undefined`
    // = accessor sans getter (`proto[h]` → undefined, capture
    // skipped, aucun throw). Si `get:undefined` est traité comme
    // champ absent chez nous → vieux getter conservé → FIRE →
    // c'est le drop §B-4(c).
    println!(
        "get:undefined → {}",
        ev(
            &mut e,
            "(function(){var o={};Object.defineProperty(o,'x',{get:function(){return 7},configurable:!0,enumerable:!0});var d=Object.getOwnPropertyDescriptor(o,'x');d.get=void 0;Object.defineProperty(o,'x',d);var d2=Object.getOwnPropertyDescriptor(o,'x');return'o.x='+o.x+'|hasGet='+d2.hasOwnProperty('get')+'|get='+(typeof d2.get)})()",
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

const WCSD: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../_scratch/webcomponents_sd.js"
);

/// Sonde §B-4(c) résiduel — le vrai `webcomponents-sd.js` (78 Ko,
/// cache daemon) : `k.removeChild=fn` pose les méthodes sur
/// `P.prototype` (`Wrapper`), puis `ubC` (kevlar) capture
/// `Cc.Wrapper.prototype.removeChild` — undefined chez nous →
/// `z.nativeRemoveChild.call` jette. On rejoue puis on inspecte :
/// la prop existe-t-elle ? extensible ? accessor vs data ?
#[test]
#[ignore = "sonde — fixture polyfill réel non versionnée"]
fn c315_webcomponents_sd_wrapper_removechild() {
    let Ok(src) = std::fs::read_to_string(WCSD) else {
        eprintln!("SKIP {WCSD} absent");
        return;
    };
    let mut e = Engine::new();
    e.install_dom(&Rc::new(RefCell::new(Dom::from_html(
        "<html><body><div id=x></div></body></html>",
    ))));
    let r = e.eval(&src);
    if let Err(t) = r {
        println!("wcsd eval THREW: {t}");
    }
    println!(
        "ShadyDOM/Wrapper → {}",
        ev(
            &mut e,
            "(function(){var SD=window.ShadyDOM;if(!SD)return'no-ShadyDOM';var W=SD.Wrapper,o=[];o.push('inUse='+SD.inUse);o.push('aS='+(typeof Element.prototype.attachShadow));o.push('gRN='+(typeof Node.prototype.getRootNode));o.push('W='+typeof W);if(W){var d=Object.getOwnPropertyDescriptor(W.prototype,'removeChild');o.push('rmD='+(d?(typeof d.value)+'/'+d.enumerable+'/'+d.writable+'/'+d.configurable:'absent'));o.push('ext='+Object.isExtensible(W.prototype));o.push('frozen='+Object.isFrozen(W.prototype));o.push('own='+Object.getOwnPropertyNames(W.prototype).length);var d2=Object.getOwnPropertyDescriptor(W.prototype,'appendChild');o.push('acD='+(d2?(typeof d2.value):'absent'));}o.push('Nrm='+typeof Node.prototype.removeChild);return o.join('|')})()",
        )
    );
    // Rejoue la capture `ubC` : `q.nativeRemoveChild = W.prototype.removeChild`
    // — si undefined ici, le wrapper installé jetterait à l'appel.
    println!(
        "ubC capture → {}",
        ev(
            &mut e,
            "(function(){var SD=window.ShadyDOM;if(!SD||!SD.Wrapper)return'no';var W=SD.Wrapper;var q={nativeRemoveChild:W.prototype.removeChild};return'capture='+(typeof q.nativeRemoveChild)})()",
        )
    );
    let nlog = e.nullread_log_drain();
    println!("nullread_log : {} lectures nulles", nlog.len());
    for l in &nlog {
        println!("  {l}");
    }
}

/// Repro ordre réel : `webcomponents-sd.js` PUIS kevlar dans le
/// même engine — `ubC` (kevlar:3721) pousse `Cc.Wrapper` =
/// `ShadyDOM.Wrapper` et capture `W.prototype.removeChild` à
/// l'install ; la fonction wrappée jette `z.nativeRemoveChild.call`
/// si la capture était undefined. Live : elle l'était.
#[allow(clippy::too_many_lines)]
#[test]
#[ignore = "sonde — fixtures réelles non versionnées"]
fn c315_wcsd_then_kevlar_ubc_capture() {
    let (Ok(sd), Ok(kv)) = (
        std::fs::read_to_string(WCSD),
        std::fs::read_to_string(KEVLAR),
    ) else {
        eprintln!("SKIP fixtures absentes");
        return;
    };
    let mut e = Engine::new();
    e.install_timers();
    e.install_xhr(Rc::new(NullXhr { next: Cell::new(0) }));
    e.install_fetch();
    e.install_dom(&Rc::new(RefCell::new(Dom::from_html(
        "<html><body><ytd-app></ytd-app><x-up></x-up><x-yt></x-yt><div id=x></div></body></html>",
    ))));
    if let Err(t) = e.eval(&sd) {
        println!("wcsd THREW: {t}");
    }
    // État juste après wcsd — avant kevlar.
    println!(
        "post-wcsd → {}",
        ev(
            &mut e,
            "(function(){var SD=window.ShadyDOM;if(!SD)return'noSD';var W=SD.Wrapper;return'W='+typeof W+'|rm='+(W?typeof W.prototype.removeChild:'?')+'|inUse='+SD.inUse})()",
        )
    );
    // Espion JS-C318 : l'instrumentation mutationnelle (patch
    // `push`/`defineProperty`/setters) a identifié puis été
    // retirée — le coupable était `flat_into`/`array_keys` qui
    // vidaient `elems` par `take` sans `put` (JS-C319, verrou
    // `c319_elems_restore`). Ne restent que les sondes en
    // lecture seule ci-dessous.
    if let Err(t) = e.eval(&kv) {
        println!("kevlar THREW: {t}");
    }
    println!(
        "yab → {}",
        ev(
            &mut e,
            "(function(){var dk=window.default_kevlar_base;var yab=dk&&dk.ev&&dk.ev.YtActionHandlerBehavior;return'yab='+(Array.isArray(yab)?'arr'+yab.length:typeof yab)+'|e0unreg='+((yab&&yab[0]&&typeof yab[0].unregisterActionMap)||'-')})()",
        )
    );
    // La capture faite par ubC : relire le même chemin.
    println!(
        "post-kevlar → {}",
        ev(
            &mut e,
            "(function(){var SD=window.ShadyDOM;var W=SD&&SD.Wrapper;if(!W)return'noW';var d=Object.getOwnPropertyDescriptor(W.prototype,'removeChild');return'rmD='+(d?(d.get?'ACC':typeof d.value):'ABSENT')+'|own='+Object.getOwnPropertyNames(W.prototype).length})()",
        )
    );
    let nlog = e.nullread_log_drain();
    println!("nullread_log : {}", nlog.len());
    for l in &nlog {
        println!("  {l}");
    }
    // 5) Le ctor `an` a-t-il atteint `T3Q(this)` ? — didRegister,
    //    hostElement, legacyOptimizations, et invocation manuelle.
    println!(
        "an-ctor state → {}",
        ev(
            &mut e,
            "(function(){var el=document.querySelector('ytd-app');if(!el)return'no-el';var o=[];o.push('didReg='+(el.polySiContext&&el.polySiContext.didRegister));o.push('lo='+(el.polySiParams&&el.polySiParams.legacyOptimizations));o.push('host='+(typeof el.hostElement));o.push('cPP='+(el.polySiContext&&typeof el.polySiContext.copyPropertiesToProto));try{el.registered();o.push('reg-call=ok')}catch(x){o.push('reg-call=THREW:'+String(x&&x.message||x).slice(0,80))}o.push('unreg2='+(typeof el.unregisterActionMap));return o.join('|')})()",
        )
    );
    // 6) Own props de l'instance + identité de classe — le ctor
    //    `r`/`an` a-t-il tourné du tout (champs posés AVANT tout
    //    throw) ?
    println!(
        "inst own → {}",
        ev(
            &mut e,
            "(function(){var el=document.querySelector('ytd-app');if(!el)return'no-el';var C=customElements.get('ytd-app');var ks=Object.getOwnPropertyNames(el);return'instof='+(el instanceof C)+'|ctorSame='+(el.constructor===C)+'|own.n='+ks.length+'|own='+ks.slice(0,25).join(',')+'|iOHE='+(el instanceof HTMLElement)})()",
        )
    );
    // 7) Le mixin Z — `lo` (legacyOptimizations) ?
    //    `_registered` a-t-il fini (__hasRegisterFinished) ?
    //    `unregisterActionMap` sur le proto de la CLASSE ?
    println!(
        "mixin Z state → {}",
        ev(
            &mut e,
            "(function(){var C=customElements.get('ytd-app');var el=document.querySelector('ytd-app');var o=[];o.push('lo='+(window.Polymer&&window.Polymer.legacyOptimizations));o.push('regFin='+(C&&C.prototype&&C.prototype.__hasRegisterFinished));o.push('_reg='+(el&&typeof el._registered));o.push('unregC='+(C&&typeof C.prototype.unregisterActionMap));o.push('behavC='+(C&&C.prototype.behaviors&&C.prototype.behaviors.length));try{el._registered&&el._registered();o.push('reg-call2=ok|unreg3='+(typeof el.unregisterActionMap))}catch(x){o.push('reg2=THREW:'+String(x&&x.message||x).slice(0,90))}return o.join('|')})()",
        )
    );
    // 8) Identité des arrays behaviors : `q.prototype.behaviors`
    //    own ? == base ? `Z53(n)` a-t-il copié les méthodes de la
    //    classe (`setPageOffset` & co) ?
    // 9) Le contenu des 16 behaviors : `unregisterActionMap` y
    //    figure-t-il ? `getOwnPropertyNames` les voit-il ?
    println!(
        "behav content → {}",
        ev(
            &mut e,
            "(function(){var C=customElements.get('ytd-app');if(!C)return'noC';var B=C.prototype.behaviors||[],o=[];for(var i=0;i<B.length;i++){var ks=Object.getOwnPropertyNames(B[i]);o.push('B'+i+'['+ks.length+']:'+(ks.indexOf('unregisterActionMap')>=0?'UNREG!':'')+ks.slice(0,5).join(','))}return o.join(' || ')})()",
        )
    );
    // 10) `q.generatedFrom` = le proto `n` passé à JZA — son
    //     propre `behaviors` contient-il YtActionHandlerBehavior ?
    //     Comparaison avec `q.prototype.behaviors`.
    println!(
        "generatedFrom → {}",
        ev(
            &mut e,
            "(function(){var C=customElements.get('ytd-app');if(!C)return'noC';var g=C.generatedFrom,o=[];o.push('g='+(typeof g));if(g){var gb=g.behaviors||[],hasU=gb.some(function(b){return b&&Object.getOwnPropertyNames(b).indexOf('unregisterActionMap')>=0});o.push('g.behav.n='+gb.length+'|g.hasU='+hasU);var qb=C.prototype.behaviors||[],qU=qb.some(function(b){return b&&Object.getOwnPropertyNames(b).indexOf('unregisterActionMap')>=0});o.push('q.hasU='+qU);o.push('g===qp:'+(g===C.prototype));o.push('g unreg='+(typeof g.unregisterActionMap))}return o.join('|')})()",
        )
    );
    // 11) `g` = generatedFrom : son ctor et son `.prototype` —
    //     l'objet proto a-t-il été remplacé après décoration ?
    //     Et quels `_.e`-behaviors sont dans `g.behaviors` ?
    println!(
        "g ctor/proto → {}",
        ev(
            &mut e,
            "(function(){var C=customElements.get('ytd-app');var g=C&&C.generatedFrom;if(!g)return'no';var o=[];var ct=g.constructor;o.push('ctor='+(ct&&ct.name));o.push('ctor.proto===g:'+(ct&&ct.prototype===g));o.push('g ownB='+g.hasOwnProperty('behaviors'));var gb=g.behaviors||[];o.push('gB.tags='+gb.map(function(b){if(!b)return'?';var ks=Object.getOwnPropertyNames(b);return ks.slice(0,2).join('~')}).join('|'));var p=Object.getPrototypeOf(g);o.push('gParent='+(p&&p.constructor&&p.constructor.name)+'|pB='+(p&&p.behaviors&&p.behaviors.length));return o.join(' | ')})()",
        )
    );
    // 12) Recherche profonde de `unregisterActionMap` dans
    //     `g.behaviors` (y compris dans les sous-arrays poussés
    //     en bloc par `_.e` sans spread).
    println!(
        "deep search → {}",
        ev(
            &mut e,
            "(function(){var C=customElements.get('ytd-app');var g=C&&C.generatedFrom;if(!g)return'no';var hits=[];var scan=function(arr,path){for(var i=0;i<arr.length;i++){var b=arr[i];if(!b)continue;var ks=Object.getOwnPropertyNames(b);if(ks.indexOf('unregisterActionMap')>=0)hits.push(path+'['+i+']');if(Array.isArray(b))scan(b,path+'['+i+']')}};scan(g.behaviors||[],'g.b');scan(C.prototype.behaviors||[],'q.b');return'hits='+(hits.join(',')||'NONE')})()",
        )
    );
    // 13) `_.V` route via `Reflect.decorate` quand présent —
    //     existe-t-il chez nous ? et le `_.e` poussé existe-t-il
    //     dans le registre `_.ev` ?
    println!(
        "V/reflect → {}",
        ev(
            &mut e,
            "(function(){var o=[];o.push('Rdec='+(typeof Reflect.decorate));o.push('Rmeta='+(typeof Reflect.getMetadata));o.push('Rdm='+(typeof Reflect.defineMetadata));try{var B=_.ev&&_.ev.YtActionHandlerBehavior;o.push('evB='+(typeof B)+'|arr='+Array.isArray(B)+'|n='+(B&&B.length));o.push('evB.unreg='+(B&&B[0]&&typeof B[0].unregisterActionMap))}catch(x){o.push('ev=THREW')}return o.join('|')})()",
        )
    );
    // 14) `_.ev` — l'alias module partagé : contient-il encore
    //     `YtActionHandlerBehavior` ? (réassigné entre modules →
    //     le décorateur a pu pousser `undefined`/autre objet).
    println!(
        "_ev state → {}",
        ev(
            &mut e,
            "(function(){var o=[];var E=window._&&window._.ev;o.push('_.='+typeof window._);o.push('ev='+typeof E);if(E&&typeof E==='object'){o.push('evKeys='+Object.keys(E).slice(0,10).join(','));o.push('evB='+typeof E.YtActionHandlerBehavior)}return o.join('|')})()",
        )
    );
    // 15) `behaviors` est-il un ACCESSOR dans la chaîne de `g`/
    //     `q` ? Un getter sans setter ferait perdre le push de
    //     `m_C` (écriture avalée ailleurs / silencieusement).
    println!(
        "behaviors desc → {}",
        ev(
            &mut e,
            "(function(){var C=customElements.get('ytd-app');if(!C)return'no';var out=[],p=C.prototype,d=0;for(;p&&d<8;d++){var x=Object.getOwnPropertyDescriptor(p,'behaviors');if(x){out.push((p.constructor&&p.constructor.name)+':'+(x.get?'ACC':'VAL')+(x.enumerable?'e':'-'))}p=Object.getPrototypeOf(p)}var g=C.generatedFrom||{},gg=Object.getPrototypeOf(g);out.push('g:'+(g.hasOwnProperty('behaviors')?(Object.getOwnPropertyDescriptor(g,'behaviors').get?'ACC':'VAL'):'inh'));var gd=gg&&Object.getOwnPropertyDescriptor(gg,'behaviors');out.push('gp:'+(gd?(gd.get?'ACC':'VAL'):'-'));return out.join('|')})()",
        )
    );
    // 16) Chaîne des CTORs — trouver la classe décorée `Ng`
    //     réelle et sa `prototype.behaviors` (celle qui a reçu
    //     le push `_.e`).
    println!(
        "ctor chain → {}",
        ev(
            &mut e,
            "(function(){var C=customElements.get('ytd-app');if(!C)return'no';var g=C.generatedFrom,qd=g&&g.constructor,o=[],f=qd,d=0;for(;f&&d<8;d++){var p=f.prototype,b=p&&p.behaviors,hasU=b&&b.some(function(x){return x&&Object.getOwnPropertyNames(x).indexOf('unregisterActionMap')>=0||(Array.isArray(x)&&x.some(function(y){return y&&Object.getOwnPropertyNames(y).indexOf('unregisterActionMap')>=0}))});o.push((f.name||'?')+':pB='+(b?b.length:'-')+(hasU?'*U*':''));f=Object.getPrototypeOf(f)}return o.join('|')})()",
        )
    );
    // 17) Identité des entrées : `Object.prototype`/`Function.
    //     prototype` figurent-ils dans `g.behaviors` ? (preuve
    //     du fallback `Reflect.getMetadata` → `n` était falsy).
    println!(
        "identity → {}",
        ev(
            &mut e,
            "(function(){var C=customElements.get('ytd-app');var g=C&&C.generatedFrom;if(!g)return'no';var o=[];var gb=g.behaviors||[];for(var i=0;i<gb.length;i++){var b=gb[i];var tag=b===Object.prototype?'OBJPROTO':b===Function.prototype?'FNPROTO':b===Array.prototype?'ARRPROTO':(b&&b.constructor&&b.constructor.name)||typeof b;o.push(i+':'+tag+(Array.isArray(b)?'/arr'+b.length:''))}return o.join('|')})()",
        )
    );
    println!(
        "behav identity → {}",
        ev(
            &mut e,
            "(function(){var C=customElements.get('ytd-app');if(!C)return'noC';var q=C.prototype,base=Object.getPrototypeOf(q),o=[];o.push('ownB='+q.hasOwnProperty('behaviors'));o.push('bB='+(base&&base.behaviors&&base.behaviors.length));o.push('same='+(q.behaviors===(base&&base.behaviors)));o.push('spo='+(typeof q.setPageOffset));o.push('oNYS='+(typeof q.onYtNavigateStart));o.push('unregB='+(base&&typeof base.unregisterActionMap));o.push('behavKeys='+Object.getOwnPropertyNames(q).filter(function(k){return/ction|avig|etach|egist/.test(k)}).join(','));return o.join('|')})()",
        )
    );
    // Mur suivant (vol 617) : `detached` kevlar:40765 lit
    // `Xcs.playlistHandlerActionMap` — `Xcs` est posé dans
    // `attached` (40763) uniquement `if(!standalone)` dans la
    // branche else. Sonde : `standalone` de l'élément +
    // `removeChild` → `detached` jette-t-il ?
    println!(
        "standalone/detached → {}",
        ev(
            &mut e,
            "(function(){var el=document.querySelector('ytd-app');if(!el)return'no-el';var o=[];o.push('sa='+el.standalone);o.push('hasA='+el.hasAttribute('standalone'));o.push('isAtt='+el.isAttached);o.push('upgD='+(typeof el.__isUpgradeDisabled)+'/'+(!!el.__isUpgradeDisabled));o.push('connT='+(typeof el.connectedCallback));o.push('discT='+(typeof el.disconnectedCallback));o.push('attT='+(typeof el.attached));o.push('detT='+(typeof el.detached));return o.join('|')})()",
        )
    );
    // Les props injectées (`nE`/`qE`) de ytd-app — `attached`
    // (40757) échoue dès `this.guideService.guideElement=` si
    // l'injection n'a pas livré → `Xcs` jamais posé → detached
    // jette (vol 617).
    println!(
        "injected props → {}",
        ev(
            &mut e,
            "(function(){var el=document.querySelector('ytd-app');if(!el)return'no-el';var o=[];['hostElement','guide','guideService','guideInnerContent','persistentPanelContainer','dynamicActionMap','boundOnTouchStart','standalone','ytActionHandlerBehavior','ytActionRouterBehavior'].forEach(function(k){o.push(k+'='+(typeof el[k]))});return o.join('|')})()",
        )
    );
    println!(
        "attached-call → {}",
        ev(
            &mut e,
            "window.__att='START';(function(){var el=document.querySelector('ytd-app');if(!el){window.__att='no-el';return}var o=[];try{el.ready();o.push('ready=ok')}catch(x){o.push('ready=THREW:'+String(x&&x.message||x).slice(0,110))}o.push('content='+(el._template&&el._template.content?el._template.content.childNodes.length:'-'));o.push('dolKeys='+(el.$?Object.keys(el.$).length:'undef'));o.push('gic='+(el.guideInnerContent===null?'NULL':el.guideInnerContent===undefined?'undef':typeof el.guideInnerContent));try{el.attached();o.push('attached=ok')}catch(x){o.push('attached=THREW:'+String(x&&x.message||x).slice(0,110))}window.__att=o.join('|')})();window.__att",
        )
    );
    println!(
        "detached-call → {}",
        ev(
            &mut e,
            "(function(){var el=document.querySelector('ytd-app');if(!el)return'no-el';var p=el.parentNode;if(!p)return'no-parent';try{p.removeChild(el);return'det=ok'}catch(x){return'det=THREW:'+String(x&&x.message||x).slice(0,120)}})()",
        )
    );
    // Le mur persiste en vol 620 — le detached réel tire via
    // `disconnectedCallback` alors que `Xcs` n'est jamais posé :
    // `attached` automatique a dû jeter en chemin (avalé par le
    // polyfill wcsd). Reproduire le chemin RÉEL : createElement
    // + appendChild → `connectedCallback` dispatcher — capturer
    // l'erreur racine + drain nullread/defprop juste après.
    println!(
        "auto-path → {}",
        ev(
            &mut e,
            "window.__ap='START';(function(){var el2=document.createElement('ytd-app');var o=[];try{document.body.appendChild(el2);o.push('append=ok')}catch(x){o.push('append=THREW:'+String(x&&x.message||x).slice(0,140))}o.push('root2='+(el2.root===el2?'self':typeof el2.root));o.push('content2='+(el2._template&&el2._template.content?el2._template.content.childNodes.length:'-'));o.push('gic2='+(typeof el2.guideInnerContent));o.push('isAtt2='+el2.isAttached);window.__ap=o.join('|')})();window.__ap",
        )
    );
    println!(
        "auto-nullread → {}",
        ev(
            &mut e,
            "JSON.stringify(__sonde_nullread ? __sonde_nullread() : [])"
        )
    );
    println!(
        "auto-defprop → {}",
        ev(
            &mut e,
            "JSON.stringify(__sonde_defprop ? __sonde_defprop() : [])"
        )
    );
}

/// Sonde JS-C319 — mur levé sur youtube live :
/// « unregisterActionMap is not a function (on `HTMLElement`) »
/// à `kevlar_base:32312:14` (`_.p.detached` de la classe
/// `ytd-app`) — cause racine : `flat_into` vidait les
/// sous-tableaux de `behaviors` (voir `c319_elems_restore`).
/// Mécanisme réel du bundle (2114) : le décorateur `_.e(B)` fait
/// `Y.behaviors.push(B)` + `defineProperty(Y,h,{get(){return
/// this}})` — `this.ytActionHandlerBehavior` === `this`, donc
/// l'appel exige `unregisterActionMap` MIXÉ dans le proto de
/// l'élément par `copyPropertiesToProto`→`qSV` (3639/3550).
/// On rejoue le vrai bundle puis on mesure où la chaîne casse :
/// `proto.behaviors` rempli ? `qSV` copié ? proto de l'instance ?
#[test]
#[ignore = "sonde — fixture bundle réel non versionnée"]
fn c318_ytd_app_unregister_actionmap() {
    let Ok(src) = std::fs::read_to_string(KEVLAR) else {
        eprintln!("SKIP {KEVLAR} absent");
        return;
    };
    let mut e = Engine::new();
    e.install_timers();
    e.install_xhr(Rc::new(NullXhr { next: Cell::new(0) }));
    e.install_fetch();
    e.install_dom(&Rc::new(RefCell::new(Dom::from_html(
        "<html><body><ytd-app></ytd-app><x-up></x-up><x-yt></x-yt><div id=x></div></body></html>",
    ))));
    if let Err(t) = e.eval(&src) {
        println!("bundle eval THREW: {t}");
    }
    // 1) La classe enregistrée : `behaviors` peuplé ? méthodes
    //    mixées sur `r.prototype` ?
    println!(
        "ytd-app class → {}",
        ev(
            &mut e,
            "(function(){var C=customElements.get&&customElements.get('ytd-app');if(typeof C!=='function')return'no-ytd-class';var p=C.prototype;var b=p.behaviors||[];return'reg=function|behav.n='+b.length+'|behav.types='+b.map(function(x){return Array.isArray(x)?'arr'+x.length:typeof x}).join(',')+'|unreg='+(typeof p.unregisterActionMap)+'|reg='+(typeof p.registerActionMap)+'|detached='+(typeof p.detached)+'|ytAHB='+(typeof p.ytActionHandlerBehavior)})()",
        )
    );
    // 2) L'instance upgradée : `ytActionHandlerBehavior===this` ?
    //    `unregisterActionMap` visible ?
    println!(
        "ytd-app inst → {}",
        ev(
            &mut e,
            "(function(){var el=document.querySelector('ytd-app');if(!el)return'no-el';return'unreg='+(typeof el.unregisterActionMap)+'|reg='+(typeof el.registerActionMap)+'|ahb===el:'+(el.ytActionHandlerBehavior===el)+'|ahbT='+(typeof el.ytActionHandlerBehavior)+'|detached='+(typeof el.detached)+'|ctor='+(el.constructor&&el.constructor.name)})()",
        )
    );
    // 3) La chaîne proto de l'instance — à quel niveau la méthode
    //    devrait être (et est-elle) ?
    println!(
        "proto chain → {}",
        ev(
            &mut e,
            "(function(){var el=document.querySelector('ytd-app');if(!el)return'no-el';var p=el,s=[];for(var d=0;p&&d<10;d++){var n=(p.constructor&&p.constructor.name)||'?';s.push(n+':'+(Object.getOwnPropertyNames(p).indexOf('unregisterActionMap')>=0?'HAS':'-'));p=Object.getPrototypeOf(p)}return s.join('|')})()",
        )
    );
    // 4) Le décorateur `_.e` a-t-il posé `get(){return this}` sur
    //    le proto de la classe décorée ?
    println!(
        "decorator desc → {}",
        ev(
            &mut e,
            "(function(){var C=customElements.get&&customElements.get('ytd-app');if(typeof C!=='function')return'no';var p=C.prototype,d=0,out=[];for(;p&&d<10;d++){var x=Object.getOwnPropertyDescriptor(p,'ytActionHandlerBehavior');if(x){out.push((p.constructor&&p.constructor.name)+':'+(x.get?'GET':'VAL')+'/'+(x.enumerable?'e':'-'))}p=Object.getPrototypeOf(p)}return out.length?out.join('|'):'no-desc'})()",
        )
    );
    let nlog = e.nullread_log_drain();
    println!("nullread_log : {}", nlog.len());
    for l in &nlog {
        println!("  {l}");
    }
}
