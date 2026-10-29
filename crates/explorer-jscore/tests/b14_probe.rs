// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! §B-14 — trous DOM/API nommés par les vols.
//!
//! - `*AttributeNS` (vol 104 boursorama : `getAttributeNS` is not
//!   a function) — spec Element : match sur la partie locale de
//!   `p:l`, `ns` nul = attr non préfixé.
//! - `FontFaceSet.add` (vol 104 boursorama) — spec : `add`
//!   retourne le set (chaînage) ; `has_method` déclare la
//!   surface pour `typeof fonts.add === 'function'`.

use explorer_jscore::{Dom, Engine};
use std::cell::RefCell;
use std::rc::Rc;

const PAGE: &str = r##"<html><body>
<svg><use id="u" xlink:href="#a" href="b" xml:lang="fr"></use></svg>
</body></html>"##;

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

#[test]
fn get_attribute_ns_matches_local_part() {
    let mut e = engine();
    e.eval("var u=document.getElementById('u')").unwrap();
    // ns donné → partie locale de `xlink:href` = `href`.
    assert_eq!(
        ev(
            &mut e,
            "u.getAttributeNS('http://www.w3.org/1999/xlink','href')"
        ),
        "#a"
    );
    // ns donné sur attr non préfixé → absent (spec : l'attr `href`
    // est no-namespace, il ne matche pas un ns xlink).
    assert_eq!(
        ev(
            &mut e,
            "u.getAttributeNS('http://www.w3.org/1999/xlink','missing') === null"
        ),
        "true"
    );
    assert_eq!(
        ev(
            &mut e,
            "u.getAttributeNS('http://www.w3.org/1999/xlink','lang')"
        ),
        "fr"
    );
}

#[test]
fn attribute_ns_null_ns_means_unprefixed() {
    let mut e = engine();
    e.eval("var u=document.getElementById('u')").unwrap();
    // `ns` null/undefined → attrs sans préfixe uniquement.
    assert_eq!(ev(&mut e, "u.getAttributeNS(null,'href')"), "b");
    assert_eq!(ev(&mut e, "u.getAttributeNS('','href')"), "b");
    // `xlink:href` a une partie locale `href` mais est préfixé —
    // il ne matche pas `ns` nul (spec : attr qualifié ≠ `href`).
    assert_eq!(ev(&mut e, "u.getAttributeNS(null,'lang') === null"), "true");
}

#[test]
fn has_and_remove_attribute_ns() {
    let mut e = engine();
    e.eval("var u=document.getElementById('u')").unwrap();
    assert_eq!(
        ev(
            &mut e,
            "u.hasAttributeNS('http://www.w3.org/1999/xlink','href') + ':' + u.hasAttributeNS(null,'missing')"
        ),
        "true:false"
    );
    ev(
        &mut e,
        "u.removeAttributeNS('http://www.w3.org/1999/xlink','href')",
    );
    assert_eq!(ev(&mut e, "u.getAttribute('xlink:href') === null"), "true");
    // L'attr non préfixé `href` survit — la suppression NS n'y
    // touche pas.
    assert_eq!(ev(&mut e, "u.getAttribute('href')"), "b");
}

#[test]
fn set_attribute_ns_sets_qualified_name() {
    let mut e = engine();
    e.eval("var u=document.getElementById('u')").unwrap();
    ev(
        &mut e,
        "u.setAttributeNS('http://www.w3.org/1999/xlink','xlink:title','T')",
    );
    assert_eq!(ev(&mut e, "u.getAttribute('xlink:title')"), "T");
}

#[test]
fn font_face_set_add_returns_set_and_is_detectable() {
    let mut e = engine();
    // Feature-detect : `typeof fonts.add === 'function'`
    // (has_method déclare la surface invoke).
    assert_eq!(ev(&mut e, "typeof document.fonts.add"), "function");
    // Spec : `add(face)` retourne le set — chaînage.
    assert_eq!(
        ev(&mut e, "var f=document.fonts; f.add({family:'x'})===f"),
        "true"
    );
    // `load`/`check` déjà servis restent détectables.
    assert_eq!(
        ev(
            &mut e,
            "typeof document.fonts.load + ':' + typeof document.fonts.check"
        ),
        "function:function"
    );
}

#[test]
fn font_face_set_is_event_target() {
    let mut e = engine();
    // impots DSFR vol 115 : `addEventListener is not a function`
    // sur `FontFaceSet` — spec : `FontFaceSet` hérite
    // `EventTarget` (loading/loadingdone/loadingerror). Rail
    // `__et` partagé via `invoke`.
    assert_eq!(
        ev(&mut e, "typeof document.fonts.addEventListener"),
        "function"
    );
    assert_eq!(
        ev(
            &mut e,
            "var f=document.fonts, n=0;\
             var cb=function(){n++};\
             f.addEventListener('loadingdone',cb);\
             f.dispatchEvent({type:'loadingdone'});\
             f.removeEventListener('loadingdone',cb);\
             f.dispatchEvent({type:'loadingdone'});\
             n"
        ),
        "1"
    );
}

#[test]
fn collections_carry_spec_to_string_tag() {
    let mut e = engine();
    // allociné vol 118 : `addEventListener is not a function
    // (on Array)` — le détecteur `i()` (motif jQuery/lodash)
    // teste `/^\[object (HTMLCollection|NodeList|Object)\]$/`
    // via `Object.prototype.toString` ; nos collections étaient
    // des Array nus → `[object Array]` → branche rejetée.
    // Spec : qSA=NodeList, byTag/byClass/children=HTMLCollection,
    // childNodes=NodeList.
    assert_eq!(
        ev(
            &mut e,
            "Object.prototype.toString.call(document.querySelectorAll('use'))"
        ),
        "[object NodeList]"
    );
    assert_eq!(
        ev(
            &mut e,
            "Object.prototype.toString.call(document.getElementsByTagName('use'))"
        ),
        "[object HTMLCollection]"
    );
    assert_eq!(
        ev(
            &mut e,
            "Object.prototype.toString.call(document.body.children)"
        ),
        "[object HTMLCollection]"
    );
    assert_eq!(
        ev(
            &mut e,
            "Object.prototype.toString.call(document.body.childNodes)"
        ),
        "[object NodeList]"
    );
    // Le détecteur verbatim d'allociné prend désormais la
    // branche par-élément (addEventListener sur les nodes).
    assert_eq!(
        ev(
            &mut e,
            r"var e=document.querySelectorAll('use');
            var i=function(x){var t=Object.prototype.toString.call(x);
              return typeof x==='object'&&/^\[object (HTMLCollection|NodeList|Object)\]$/.test(t)&&x.length!==undefined&&(x.length===0||typeof x[0]==='object'&&x[0].nodeType>0)};
            i(e)"
        ),
        "true"
    );
}

#[test]
fn html_document_interface_exists_and_chains() {
    let mut e = engine();
    // orange vol 109 : `HTMLDocument is not defined` tuait
    // cookie-monster woopic. Spec : `document` →
    // `HTMLDocument.prototype` → `Document.prototype`.
    assert_eq!(ev(&mut e, "typeof HTMLDocument"), "function");
    assert_eq!(ev(&mut e, "document instanceof HTMLDocument"), "true");
    // Transitivité préservée : Document/Node/EventTarget.
    assert_eq!(ev(&mut e, "document instanceof Document"), "true");
    assert_eq!(ev(&mut e, "document instanceof Node"), "true");
    assert_eq!(ev(&mut e, "document.constructor === HTMLDocument"), "true");
    // `implementation.createHTMLDocument` hérite aussi
    // `HTMLDocument.prototype` (spec).
    assert_eq!(
        ev(
            &mut e,
            "document.implementation.createHTMLDocument('t') instanceof HTMLDocument"
        ),
        "true"
    );
    // Non-régression : le feature-detect github sur le proto
    // Document reste vrai.
    assert_eq!(
        ev(&mut e, "'adoptedStyleSheets' in Document.prototype"),
        "true"
    );
}

#[test]
fn insert_before_adopts_foreign_document_nodes() {
    let mut e = engine();
    // appconsent vol 110 : `insertBefore: not a node` — le
    // conteneur styled-components vit dans un autre document ;
    // spec `[[adopt]]` : le nœud change de document (move).
    assert_eq!(
        ev(
            &mut e,
            "var d2=document.implementation.createHTMLDocument('t');\
             var el=d2.createElement('div'); el.id='adopted';\
             el.textContent='X'; document.body.insertBefore(el,null);\
             document.getElementById('adopted')===el"
        ),
        "true"
    );
    // `el` rebindé : il désigne le nœud dans le document cible.
    assert_eq!(ev(&mut e, "el.ownerDocument === document"), "true");
    // Sous-arbre entier adopté + appendChild idem.
    assert_eq!(
        ev(
            &mut e,
            "var d3=document.implementation.createHTMLDocument('t2');\
             var p=d3.createElement('p'); var c=d3.createElement('b');\
             c.textContent='deep'; p.appendChild(c);\
             document.body.appendChild(p);\
             document.querySelector('body b').textContent"
        ),
        "deep"
    );
    // Arg non-nœud : `TypeError` typé préservé.
    assert_eq!(
        ev(
            &mut e,
            "try{document.body.insertBefore({},null);'NO'}\
             catch(er){/node/.test(er.message)?'typed':'?'}"
        ),
        "typed"
    );
}

#[test]
fn style_decl_only_serves_css_property_names() {
    let mut e = engine();
    // jvc vol 103 : `hasOwnProperty is not a function` sur
    // `CSSStyleDeclaration` — le named-getter servait `""` pour
    // TOUT nom, avalant `Object.prototype` et les méthodes host.
    // Spec : le getter nommé ne couvre que les propriétés CSS.
    e.eval("var s=document.body.style; s.color='red'").unwrap();
    assert_eq!(
        ev(
            &mut e,
            "typeof s.hasOwnProperty + ':' + s.hasOwnProperty('color')"
        ),
        "function:false"
    );
    // Méthodes host lisibles en valeur (`typeof` + capture).
    assert_eq!(ev(&mut e, "typeof s.setProperty"), "function");
    assert_eq!(ev(&mut e, "typeof s.toString"), "function");
    // Propriété CSS : non déclarée → `""`, déclarée → valeur.
    assert_eq!(ev(&mut e, "s.zIndex === ''"), "true");
    assert_eq!(ev(&mut e, "s.color"), "red");
    // Lecture symbole → `undefined`, pas `""`.
    assert_eq!(ev(&mut e, "s[Symbol.iterator] === undefined"), "true");
    // `invoke` des méthodes host inchangé.
    assert_eq!(
        ev(
            &mut e,
            "s.setProperty('x-foo','1'); s.getPropertyValue('x-foo')"
        ),
        "1"
    );
}

#[test]
fn insert_adjacent_position_is_ascii_case_insensitive() {
    let mut e = engine();
    // jvc vol 114 : `insertAdjacentHTML("beforeEnd", …)` (jQuery
    // camelCase) jetait `bad position` — spec : les quatre
    // positions sont ASCII case-insensitive.
    assert_eq!(
        ev(
            &mut e,
            "var t=document.body;\
             t.insertAdjacentHTML('beforeEnd','<b id=b1>B</b>');\
             t.insertAdjacentHTML('AfterBegin','<i id=i1>I</i>');\
             t.insertAdjacentText('beforeEND','TX');\
             var k=t.childNodes;\
             (k[0].id)+'|'+(k[k.length-2].id)+'|'+(typeof k[k.length-1].data==='string'?k[k.length-1].data:'?')"
        ),
        "i1|b1|TX"
    );
    // Variante élément : `insertAdjacentElement` même rail.
    assert_eq!(
        ev(
            &mut e,
            "var c=document.createElement('u');\
             document.getElementById('b1').insertAdjacentElement('AfterEnd',c);\
             document.getElementById('b1').nextSibling===c"
        ),
        "true"
    );
}

#[test]
fn elements_have_tag_specific_dom_interfaces() {
    // sephora vol 121 : invariant Next.js
    // `document.currentScript instanceof HTMLScriptElement` —
    // tout élément portait `HTMLElement.prototype`, le proto
    // tag-spécifique manquait. Spec : `<script>` →
    // `HTMLScriptElement.prototype` → `HTMLElement` →
    // `Element` → `Node` → `EventTarget`.
    let mut e = Engine::new();
    e.install_dom(&Rc::new(RefCell::new(Dom::from_html(
        "<html><body><script>1</script><div id=d>x</div><a href='#'>l</a></body></html>",
    ))));
    assert_eq!(
        ev(
            &mut e,
            "var s=document.getElementsByTagName('script')[0];\
             (s instanceof HTMLScriptElement)+'|'\
             +(s instanceof HTMLElement)+'|'\
             +(s instanceof Element)+'|'\
             +(s instanceof Node)"
        ),
        "true|true|true|true"
    );
    assert_eq!(
        ev(
            &mut e,
            "(document.getElementById('d') instanceof HTMLDivElement)+'|'\
             +(document.getElementsByTagName('a')[0] instanceof HTMLAnchorElement)+'|'\
             +(document.getElementById('d') instanceof HTMLAnchorElement)"
        ),
        "true|true|false"
    );
    // Chaîne proto spec : `HTML*Element.prototype` hérite
    // `HTMLElement.prototype` (pas `Element` direct — sinon
    // `el instanceof HTMLElement` deviendrait faux).
    assert_eq!(
        ev(
            &mut e,
            "Object.getPrototypeOf(HTMLScriptElement.prototype)===HTMLElement.prototype"
        ),
        "true"
    );
    // `el.constructor` suit le proto tag-spécifique.
    assert_eq!(
        ev(
            &mut e,
            "(document.getElementById('d').constructor===HTMLDivElement)+'|'\
             +(document.getElementsByTagName('script')[0].constructor===HTMLScriptElement)"
        ),
        "true|true"
    );
    // Tag hors table → `HTMLElement` (spec rend
    // `HTMLUnknownElement`, non modélisé).
    assert_eq!(
        ev(
            &mut e,
            "var u=document.createElement('x-foo');u instanceof HTMLElement"
        ),
        "true"
    );
}

#[test]
fn scheduler_post_task_resolves_via_timer() {
    // dna vol 126 : `postTask` Scheduler API absent — spec :
    // `scheduler.postTask(cb, opts)` → promesse résolue au
    // retour de `cb` après `opts.delay` ms ; `signal` aborté
    // → rejet `AbortError` ; `cb` qui jette → rejet.
    let mut e = engine();
    assert_eq!(ev(&mut e, "typeof scheduler.postTask"), "function");
    // cb non-appelable → TypeError spec.
    assert_eq!(
        ev(
            &mut e,
            "try{scheduler.postTask(1);'no'}catch(x){x instanceof TypeError}"
        ),
        "true"
    );
    // Résolution au retour du callback via la file de timers.
    e.eval("var r='';\
            scheduler.postTask(function(){return 7}).then(function(v){r='ok:'+v},function(x){r='rej:'+x.name});\
            scheduler.postTask(function(){throw 'boom'}).then(function(){r='no'},function(x){r=r+'|thrown'});")
        .unwrap_or_else(|x| panic!("{x:?}"));
    e.run_due_timers().expect("timers");
    e.run_microtasks().expect("microtasks");
    assert_eq!(ev(&mut e, "r"), "ok:7|thrown");
    // `signal` déjà aborté → rejet `AbortError` immédiat.
    assert_eq!(
        ev(
            &mut e,
            "var s2='';\
             scheduler.postTask(function(){}, {signal:AbortSignal.abort()}).then(function(){s2='no'},function(x){s2=x.name});\
             'queued'"
        ),
        "queued"
    );
    e.run_microtasks().expect("microtasks");
    assert_eq!(ev(&mut e, "s2"), "AbortError");
    // `priority` accepté (non ordonnancé — déviation bornée).
    assert_eq!(
        ev(
            &mut e,
            "scheduler.postTask(function(){return 1},{priority:'background',delay:0}) instanceof Promise"
        ),
        "true"
    );
}

#[test]
fn xpath_evaluate_common_axes_and_predicates() {
    // insee vol 122 : `evaluate: unsupported XPath` tuait le
    // script — la grammaire ne couvrait que `//tag[@a]`/`[n]`.
    // Étendue : axes `ancestor::`/`self::`/`following-sibling::`/
    // `preceding-sibling::`/`parent::`/`descendant*`/`attribute::`,
    // prédicats `contains`/`starts-with`/`ends-with`/`text()`/
    // `last()`/`position()`/`not()`, union `|`. Le reste →
    // résultat vide + warn console (dégradé, plus de throw).
    let mut e = Engine::new();
    e.install_dom(&Rc::new(RefCell::new(Dom::from_html(
        "<html><body><div id=a class='x'><p>t1</p><p>t2</p></div>\
         <div id=b class='xy'><span>S</span></div>\
         <ul><li>1</li><li>2</li><li>3</li></ul></body></html>",
    ))));
    assert_eq!(
        ev(
            &mut e,
            "document.evaluate('//p',document,null,7,null).snapshotLength"
        ),
        "2"
    );
    assert_eq!(
        ev(
            &mut e,
            "document.evaluate(\"//div[@class='x']\",document,null,7,null).snapshotLength"
        ),
        "1"
    );
    assert_eq!(
        ev(
            &mut e,
            "document.evaluate(\"//div[contains(@class,'x')]\",document,null,7,null).snapshotLength"
        ),
        "2"
    );
    assert_eq!(
        ev(
            &mut e,
            "document.evaluate('//span/ancestor::div',document,null,7,null).snapshotLength"
        ),
        "1"
    );
    assert_eq!(
        ev(
            &mut e,
            "document.evaluate('//div[@id=\"a\"]/following-sibling::div',document,null,7,null).snapshotLength"
        ),
        "1"
    );
    assert_eq!(
        ev(
            &mut e,
            "document.evaluate('//p|//span',document,null,7,null).snapshotLength"
        ),
        "3"
    );
    assert_eq!(
        ev(
            &mut e,
            "document.evaluate('//li[last()]',document,null,7,null).snapshotItem(0).textContent"
        ),
        "3"
    );
    assert_eq!(
        ev(
            &mut e,
            "document.evaluate(\"//p[text()='t2']\",document,null,7,null).snapshotLength"
        ),
        "1"
    );
    // XPath valide hors sous-ensemble → résultat vide (warn
    // console émis), plus de `TypeError`.
    assert_eq!(
        ev(
            &mut e,
            "document.evaluate('//x:y',document,null,7,null).snapshotLength"
        ),
        "0"
    );
    // `id()`/`count()`/`..`/`.` inchangés.
    assert_eq!(
        ev(
            &mut e,
            "document.evaluate('count(//li)',document,null,1,null).numberValue"
        ),
        "3"
    );
}

#[test]
fn navigator_java_enabled_returns_false() {
    // maif vol 122 : `javaEnabled is not a function` — méthode
    // spec (dépréciée) lue par les feature-detects ; spec :
    // toujours `false` (aucun plugin Java).
    let mut e = engine();
    assert_eq!(
        ev(
            &mut e,
            "typeof navigator.javaEnabled+'|'+navigator.javaEnabled()"
        ),
        "function|false"
    );
    // ryanair vol 127 : `navigator.vendor.toString()` tuait le
    // boot (`toString of Undefined`) → zone incomplète →
    // `MutationObserver: callback requis` en cascade. `vendor`
    // vide (spec — non-Google honnête) mais *défini*.
    assert_eq!(
        ev(
            &mut e,
            "typeof navigator.vendor+'|'+navigator.vendor.toString()+'|'\
             +navigator.appCodeName+'|'+navigator.product+'|'\
             +navigator.productSub+'|'+(typeof navigator.vendorSub)"
        ),
        "string||Mozilla|Gecko|20030107|string"
    );
}

// SG vol 128 : `hasAttribute is not a function (on HTMLDocument)`
// — le marcheur d'ancêtres (`parentNode`×n) se gardait sur
// `"body"===r.localName` ; `localName` absent → remontée jusqu'au
// `document` → TypeError. Spec : `localName` minuscule sur les
// éléments, `null` sur les nœuds non-élément.
#[test]
fn local_name_stops_ancestor_walkers_at_body() {
    let mut e = engine();
    assert_eq!(ev(&mut e, "document.body.localName"), "body");
    assert_eq!(ev(&mut e, "document.documentElement.localName"), "html");
    assert_eq!(
        ev(&mut e, "document.body.firstChild.localName === null"),
        "true"
    );
    // Repro du garde-fou SG : `parentNode` monte jusqu'à `body`,
    // localName l'arrête avant le `document` (avant : TypeError).
    assert_eq!(
        ev(
            &mut e,
            "(()=>{const o=(e,t)=>{for(;t-->0;)if(!(e=e.parentNode))return null;return e};\
             for(let t=0;t<10;t++){const r=o(document.getElementById('u'),t);\
             if(r===null||'body'===r.localName)return 'stop@'+t;\
             if(r===document)return 'doc@'+t}return 'never'})()"
        ),
        "stop@2"
    );
}
