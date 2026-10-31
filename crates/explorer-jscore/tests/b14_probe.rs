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
fn font_face_set_setlike_tracks_added_faces() {
    let mut e = engine();
    // §B-14 vol 222 excalidraw — `fonts.has(face)` absent
    // cassait le boot ; un `has` toujours faux re-chargerait
    // la fonte en boucle. Setlike : identité, dedup, delete,
    // clear, size, unicité entre accès `document.fonts`.
    assert_eq!(ev(&mut e, "typeof document.fonts.has"), "function");
    assert_eq!(
        ev(
            &mut e,
            "var ff={family:'x'};\
             document.fonts.add(ff);\
             document.fonts.has(ff)+':'+document.fonts.size"
        ),
        "true:1"
    );
    assert_eq!(
        ev(
            &mut e,
            "var ff={family:'x'}, out=[];\
             document.fonts.clear();\
             document.fonts.add(ff); document.fonts.add(ff);\
             document.fonts.forEach(function(v){out.push(v===ff)});\
             out.length+':'+out[0]"
        ),
        "1:true"
    );
    assert_eq!(
        ev(
            &mut e,
            "var ff={family:'x'};\
             document.fonts.clear();\
             document.fonts.add(ff);\
             document.fonts.delete(ff)+':'+document.fonts.has(ff)+':'+document.fonts.size"
        ),
        "true:false:0"
    );
    assert_eq!(
        ev(
            &mut e,
            "document.fonts.add({});\
             document.fonts.clear();\
             document.fonts.size"
        ),
        "0"
    );
}

#[test]
fn import_node_clones_foreign_document_nodes() {
    let mut e = engine();
    // §B-14 EXPLOR78 usnews ×5 — `importNode: nœud requis`
    // sur nœud d'un autre document (template jQuery). Spec :
    // clone sans détacher ni rebind — deep ET shallow.
    assert_eq!(
        ev(
            &mut e,
            "var d2=document.implementation.createHTMLDocument('x');\
             d2.body.innerHTML='<div id=a><span>hi</span></div>';\
             var src=d2.getElementById('a');\
             var c=document.importNode(src,true);\
             c.tagName+':'+c.childNodes.length+':'+src.childNodes.length"
        ),
        "DIV:1:1"
    );
    // shallow : pas d'enfants clonés, source intacte.
    assert_eq!(
        ev(
            &mut e,
            "var d2=document.implementation.createHTMLDocument('x');\
             d2.body.innerHTML='<div id=a><span>hi</span></div>';\
             var src=d2.getElementById('a');\
             var c=document.importNode(src,false);\
             c.tagName+':'+c.childNodes.length+':'+src.childNodes.length"
        ),
        "DIV:0:1"
    );
    // le clone est insérable dans le document hôte.
    assert_eq!(
        ev(
            &mut e,
            "var d2=document.implementation.createHTMLDocument('x');\
             d2.body.innerHTML='<p>z</p>';\
             var c=document.importNode(d2.body.firstChild,true);\
             document.body.appendChild(c);\
             document.body.lastChild.tagName"
        ),
        "P"
    );
    // §B-14 REVALID3 usnews — `importNode(document entier
    // étranger)` (jQuery `parseHTML` importe le document
    // auxiliaire) : le `DocumentHost` étranger clone sa
    // racine, sans rejet « nœud requis ». Modèle d'arène :
    // la racine `#document` est un `Element` — le clone en
    // hérite (nodeType 1, `getElementsByTagName` vivant) ;
    // le `nodeType` 9 spec exigerait un `NodeKind::Document`.
    assert_eq!(
        ev(
            &mut e,
            "var d2=document.implementation.createHTMLDocument('x');\
             d2.body.innerHTML='<b>w</b>';\
             var c=document.importNode(d2,true);\
             c.nodeType+':'+c.getElementsByTagName('b').length"
        ),
        "1:1"
    );
    // §B-14 REVALID5 usnews — spec : `importNode(Attr)` rend
    // un Attr cloné (nos porteurs `{nodeType:2}` ne sont pas
    // des `Host`).
    assert_eq!(
        ev(
            &mut e,
            "var a=document.createAttribute('data-x');\
             a.value='v';\
             var b=document.importNode(a);\
             b.nodeType+':'+b.name+':'+b.value"
        ),
        "2:data-x:v"
    );
    // Message diagnostique : le type reçu nomme le bras au
    // prochain vol (TypeError maintenu — parité Chrome pour
    // `importNode(undefined)`).
    assert_eq!(
        ev(
            &mut e,
            "try{document.importNode(undefined);'no'}\
             catch(x){x.message.indexOf('reçu=undefined')>=0}"
        ),
        "true"
    );
}

#[test]
fn attr_node_rails_roundtrip() {
    let mut e = engine();
    // §B-14 EXPLOR74 npr ×4 — `createAttribute is not a
    // function` + rails `*AttributeNode` des polyfills.
    assert_eq!(ev(&mut e, "typeof document.createAttribute"), "function");
    assert_eq!(
        ev(
            &mut e,
            "var a=document.createAttribute('data-x');\
             a.nodeType+':'+a.name+':'+(a.value==='')"
        ),
        "2:data-x:true"
    );
    assert_eq!(
        ev(
            &mut e,
            "var el=document.createElement('div');\
             var a=document.createAttribute('data-y');\
             a.value='42';\
             (el.setAttributeNode(a)===null)+':'+el.getAttribute('data-y')"
        ),
        "true:42"
    );
    assert_eq!(
        ev(
            &mut e,
            "var el=document.createElement('div');\
             el.setAttribute('data-y','1');\
             var old=document.createAttribute('data-y'); old.value='2';\
             var rep=el.setAttributeNode(old);\
             rep.value+':'+el.getAttribute('data-y')"
        ),
        "1:2"
    );
    assert_eq!(
        ev(
            &mut e,
            "var el=document.createElement('div');\
             el.setAttribute('id','q');\
             var a=el.getAttributeNode('id');\
             (a&&a.value)+':'+typeof el.getAttributeNode('zz')"
        ),
        "q:object"
    );
    // removeAttributeNode rend l'attr retiré ; absent → NotFoundError.
    assert_eq!(
        ev(
            &mut e,
            "var el=document.createElement('div');\
             el.setAttribute('id','q');\
             var a=el.getAttributeNode('id');\
             var r=el.removeAttributeNode(a);\
             (r===a)+':'+el.hasAttribute('id')"
        ),
        "true:false"
    );
}

#[test]
fn scroll_methods_exist_and_are_noop() {
    let mut e = engine();
    // §B-14 EXPLOR68 iltalehti — `scrollTo` absent ; lazy-
    // loaders référencent la méthode librement (no-op honnête,
    // le scroll page vit côté session).
    assert_eq!(
        ev(
            &mut e,
            "typeof scrollTo+':'+typeof scroll+':'+typeof scrollBy"
        ),
        "function:function:function"
    );
    assert_eq!(
        ev(
            &mut e,
            "var el=document.createElement('div');\
             typeof el.scrollTo+':'+typeof el.scroll+':'+typeof el.scrollBy"
        ),
        "function:function:function"
    );
    // Args coords ou objet options acceptés, jamais d'erreur.
    assert_eq!(
        ev(
            &mut e,
            "scrollTo(0,100); scroll({top:50,behavior:'smooth'}); 'ok'"
        ),
        "ok"
    );
}

#[test]
fn intersection_observer_threshold_typed_array() {
    let mut e = engine();
    // §B-14 EXPLOR73 CNN — `threshold` passé en
    // `Float32Array` (spec `sequence<double>` = iterable) :
    // notre rejet « not a number » tuait le constructeur.
    assert_eq!(
        ev(
            &mut e,
            "var io=new IntersectionObserver(function(){},{threshold:new Float32Array([0,0.5,1])});\
             io.thresholds.length+':'+io.thresholds[1]"
        ),
        "3:0.5"
    );
    // number / array / undefined inchangés.
    assert_eq!(
        ev(
            &mut e,
            "new IntersectionObserver(function(){},{threshold:0.25}).thresholds[0]"
        ),
        "0.25"
    );
    // §B-14 REVALID3 CNN — `threshold is not a number` sur
    // valeur primitive : WebIDL `double` = `ToNumber` — une
    // string est acceptée ; `sequence<double>` accepte tout
    // iterable (`Set`).
    assert_eq!(
        ev(
            &mut e,
            "new IntersectionObserver(function(){},{threshold:'0.5'}).thresholds[0]"
        ),
        "0.5"
    );
    assert_eq!(
        ev(
            &mut e,
            "new IntersectionObserver(function(){},{threshold:new Set([0,0.75])}).thresholds.length"
        ),
        "2"
    );
}

#[test]
fn font_face_set_is_singleton_per_document() {
    let mut e = engine();
    // Spec : `document.fonts` est l'unique `FontFaceSet` du
    // document — l'état setlike persiste entre accès.
    assert_eq!(ev(&mut e, "document.fonts===document.fonts"), "true");
    assert_eq!(
        ev(
            &mut e,
            "var ff={a:1};\
             document.fonts.add(ff); document.fonts.has(ff)"
        ),
        "true"
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

// sfr vol 131 : `node is from a different document` en rejet de
// promesse — `parseHTML` crée des nœuds hors-document puis
// `mainContainer.append(frag)`. JS-C185 câblait l'adoption sur
// `appendChild`/`insertBefore`/`replaceChild` mais `norm_children`
// (`append`/`prepend`/`before`/`after`/`replaceWith`) rejetait
// encore. Spec : adoption partout (move + rebind de la référence).
#[test]
fn append_family_adopts_foreign_document_nodes() {
    let mut e = engine();
    let html = "(()=>{
      const d=document.implementation.createHTMLDocument('x');
      const c=document.createElement('div');
      const frag=d.createElement('div'); frag.id='r';
      const p=d.createElement('b'); const a=d.createElement('i');
      c.append(frag); c.prepend(p); frag.before(a);
      const ok=frag.parentNode===c && frag===c.childNodes[2];
      const rw=d.createElement('u'); frag.replaceWith(rw);
      return [ok, c.childNodes.length,
              rw.parentNode===c && rw===c.childNodes[2] && frag.parentNode===null,
              d.getElementById('r')===null].join('|');
    })()";
    assert_eq!(ev(&mut e, html), "true|3|true|true");
}

// axa/laposte vol 131 : `addEventListener of Undefined` en
// exception — Dynatrace `ruxitagentjs` patche
// `Worker.prototype.addEventListener.call(w, …)` ; le ctor
// `Worker` n'avait pas de `prototype` → l'agent entier mourait.
// Spec WebIDL : `Worker.prototype` expose les méthodes IDL et
// hérite `EventTarget.prototype`.
#[test]
fn worker_prototype_exposes_event_target_surface() {
    let mut e = engine();
    assert_eq!(
        ev(
            &mut e,
            "typeof Worker.prototype.addEventListener+'|'\
             +typeof Worker.prototype.postMessage+'|'\
             +typeof Worker.prototype.terminate+'|'\
             +(Object.getPrototypeOf(Worker.prototype)===EventTarget.prototype)+'|'\
             +(Worker.prototype.constructor===Worker)"
        ),
        "function|function|function|true|true"
    );
    // `performance.setResourceTimingBufferSize`/
    // `clearResourceTimings` — no-ops honnêtes du rail perf
    // (l'agent ruxit les appelle au boot).
    assert_eq!(
        ev(
            &mut e,
            "typeof performance.setResourceTimingBufferSize+'|'\
             +typeof performance.clearResourceTimings"
        ),
        "function|function"
    );
}

// orpi vol 136 : `replace of Undefined` tuait le chunk swipers
// — pluralizer i18n `document.documentElement.lang.replace(
// "-","_")` ; `lang`/`dir` (globals `HTMLElement`, spec IDL)
// n'étaient pas réfléchis → `undefined` → TypeError → chunk mort
// → `Translator` jamais défini → `Translator is not defined`.
// Attr absent → `""` (spec — jamais `undefined`).
#[test]
fn lang_dir_reflected_as_strings_on_elements() {
    let mut e = engine();
    // fixture : pas d'attribut `lang` → `""` (spec).
    assert_eq!(
        ev(&mut e, "document.documentElement.lang.replace('-','_')"),
        ""
    );
    // attribut présent → valeur reflétée.
    e.eval("document.documentElement.setAttribute('lang','fr')")
        .unwrap();
    assert_eq!(
        ev(&mut e, "document.documentElement.lang.replace('-','_')"),
        "fr"
    );
    assert_eq!(
        ev(
            &mut e,
            "const d=document.createElement('div');\
             [d.lang,d.dir,typeof d.lang].join('|')"
        ),
        "||string"
    );
    // écriture → attribut
    assert_eq!(
        ev(
            &mut e,
            "const d=document.createElement('div');d.lang='en-GB';\
             d.getAttribute('lang')"
        ),
        "en-GB"
    );
}

// yves-rocher vol 136 : `startTime of Undefined` — bv.js fait
// `performance.measure(e)` puis `getEntriesByType("measure")
// .at(-1).startTime` ; nos `mark`/`measure` étaient des no-ops
// → tableau vide → `.startTime` sur `undefined`. User Timing L2
// : entrées réelles (borné : `navigation`/`resource` restent
// `[]` — pas de timeline réseau/nav).
#[test]
fn user_timing_entries_are_stored_and_filterable() {
    let mut e = engine();
    assert_eq!(
        ev(
            &mut e,
            "(()=>{performance.mark('m0');performance.measure('x','m0');\
             performance.measure('y');\
             const ms=performance.getEntriesByType('measure');\
             const last=ms[ms.length-1];\
             return [last.entryType,typeof last.startTime,last.name,\
             performance.getEntriesByName('x','measure').length,\
             performance.getEntries().length,\
             performance.getEntriesByType('navigation').length].join('|')})()"
        ),
        "measure|number|y|1|4|1"
    );
    // clear* : filtre par nom puis vidage complet.
    assert_eq!(
        ev(
            &mut e,
            "performance.clearMeasures('y');\
             performance.getEntriesByType('measure').length+'|'+\
             (performance.clearMarks(),performance.getEntriesByType('mark').length)"
        ),
        "1|0"
    );
}

// JS-C203 — GC : les `Value` retenues dans les hosts DOM
// (`NodeCache`, `implementation`, `EventState.props`, records
// IDB) doivent être tracées — sinon le mark-sweep rend des
// `ObjId` morts : « `x` is not a function (on ?) » /
// `of Undefined` (assemblee vol 137 : jQuery `implementation.
// createHTMLDocument` tué après une collecte).
#[test]
fn gc_preserves_cached_dom_wrappers() {
    let mut e = engine();
    // Wrappers créés puis **lâchés** : seuls les caches host les
    // retiennent — c'est eux que le mark-sweep doit tracer.
    e.eval(
        "document.getElementById('u');document.implementation;\
            document.querySelector('svg')",
    )
    .unwrap();
    let _ = e.collect();
    // Le wrapper caché est toujours vivant : identité + méthodes.
    assert_eq!(ev(&mut e, "document.getElementById('u').localName"), "use");
    assert_eq!(
        ev(
            &mut e,
            "document.getElementById('u').getAttribute('href')+'|'+\
             (document.getElementById('u')===document.querySelector('svg use'))"
        ),
        "b|true"
    );
    // `implementation` lazy-cachée : méthode encore fonction.
    assert_eq!(
        ev(
            &mut e,
            "document.implementation.createHTMLDocument('t').title"
        ),
        "t"
    );
    // Après un second GC + allocation fraîche, tout tient.
    e.eval("for(let i=0;i<50;i++)({a:i})").unwrap();
    let _ = e.collect();
    // `use` = tag SVG — le nom qualifié garde sa casse
    // (spec foreign elements, JS-C233).
    assert_eq!(ev(&mut e, "document.getElementById('u').tagName"), "use");
}

// JS-C204 — surface `X.prototype` WebIDL des ctors
// `EventTarget`/`Observer` : les instruments (ruxit
// mondialrelay vol 140, zone.js) patchent
// `X.prototype.addEventListener.call(x, …)` au boot — sans
// proto exposé la lecture jetait `of Undefined` et tuait
// l'agent entier.
#[test]
fn event_target_ctor_prototypes_are_exposed() {
    let mut e = engine();
    // Présence du prototype + méthodes IDL.
    assert_eq!(
        ev(
            &mut e,
            "['EventSource','SharedWorker','MessageChannel','MessagePort','MutationObserver','Worker']\
             .map(n=>n+':'+typeof globalThis[n].prototype+'/'+(globalThis[n].prototype&&typeof globalThis[n].prototype.addEventListener||'-')).join('|')"
        ),
        // Spec : `MessageChannel`/`MutationObserver` ne sont
        // pas des `EventTarget` — `addEventListener` absent
        // est le comportement attendu.
        "EventSource:object/function|SharedWorker:object/function|MessageChannel:object/undefined|MessagePort:object/function|MutationObserver:object/undefined|Worker:object/function"
    );
    // Chaînage `EventTarget.prototype` (spec).
    assert_eq!(
        ev(
            &mut e,
            "['EventSource','SharedWorker','MessagePort']\
             .map(n=>Object.getPrototypeOf(globalThis[n].prototype)===EventTarget.prototype)\
             .join(',')"
        ),
        "true,true,true"
    );
    // Greffe `.call` sur un port vivant (motif ruxit).
    assert_eq!(
        ev(
            &mut e,
            "(()=>{const c=new MessageChannel();\
             let hit=false;\
             MessagePort.prototype.addEventListener.call(c.port1,'message',()=>hit=true);\
             c.port2.postMessage(1);\
             return (c.port1 instanceof MessagePort)+'|'+(typeof c.port1.postMessage)})()"
        ),
        "true|function"
    );
    // MutationObserver : proto + instanceof.
    assert_eq!(
        ev(
            &mut e,
            "(()=>{const mo=new MutationObserver(()=>{});\
             return (mo instanceof MutationObserver)+'|'+\
             (typeof MutationObserver.prototype.observe)+'|'+\
             (typeof MutationObserver.prototype.takeRecords)})()"
        ),
        "true|function|function"
    );
}

/// JS-C205 — `Cannot call a class as a function` /
/// `class constructor cannot be invoked without new`
/// (materiel/rdc vol 160, bras récurrent ×3 sites).
///
/// (a) `Reflect.construct(Boolean, [], fn)` sous newTarget
/// étranger : l'objet frais doit recevoir le slot interne
/// (`__prim`) — sans lui `Boolean.prototype.valueOf.call`
/// jette, le feature-detect Babel `_isNativeReflectConstruct`
/// tombe à `false` et `_callSuper` rebascule sur
/// `e.apply(t, r)` = appel de classe sans `new` → `TypeError`.
///
/// (b) `new (ctor.bind(…))` : la bound fn n'a pas de
/// `.prototype` — les moteurs réels résolvent le proto à
/// travers la chaîne bound (Angular DI
/// `new (t.bind.apply(t,[null].concat(i)))`), sinon
/// `this instanceof ctor` est faux et `_classCallCheck` jette.
#[test]
fn reflect_construct_foreign_newtarget_and_bound_ctor() {
    let mut e = engine();
    // (a) slot interne posé sous newTarget étranger — c'est le
    // test exact du helper Babel `_isNativeReflectConstruct`.
    assert_eq!(
        ev(
            &mut e,
            "var A=function(){try{\
             var t=!Boolean.prototype.valueOf.call(Reflect.construct(Boolean,[],(function(){})))\
             }catch(e){}return!!t};A()"
        ),
        "true"
    );
    assert_eq!(
        ev(
            &mut e,
            "class K2{constructor(){this.k=3}};\
             var rk=Reflect.construct(K2,[],K2.bind(null));\
             rk.k+'|'+(rk instanceof K2)"
        ),
        "3|true"
    );
    // (b) `new (babelClass.bind(…))` — `_classCallCheck` passe.
    assert_eq!(
        ev(
            &mut e,
            "var E=(function(){function E(a){\
             if(!(this instanceof E))throw new TypeError('Cannot call a class as a function');\
             this.a=a}return E})();\
             var b=new(E.bind.apply(E,[null,42]))();\
             b.a+'|'+(b instanceof E)"
        ),
        "42|true"
    );
    // Chaîne bound×2 résolue + `.prototype` propre posé sur la
    // bound fn honoré en priorité.
    assert_eq!(
        ev(
            &mut e,
            "var G=(function(){function G(){this.g=7}return G})();\
             var bg=new(G.bind(null).bind(null))();\
             bg.g+'|'+(bg instanceof G)"
        ),
        "7|true"
    );
    assert_eq!(
        ev(
            &mut e,
            "function H(){this.h=1}; var bh=H.bind(null);\
             bh.prototype={x:9}; var ih=new bh(); ih.h+'|'+ih.x"
        ),
        "1|9"
    );
}

// cowcotland vol 172 (cloudflare-insights) : `addEventListener is
// not a function (on function)` — `window.navigation` absent, un
// `function navigation()` sloppy la remplaçait ; `navigation.
// addEventListener('navigate',…)` mourrait sur le receveur
// fonction. Navigation API WHATWG : `Navigation` est un
// `EventTarget` non clobbable par déclaration sloppy.
#[test]
fn window_navigation_is_event_target_and_sloppy_proof() {
    let mut e = engine();
    // Surface : EventTarget + méthodes de déplacement.
    assert_eq!(
        ev(
            &mut e,
            "typeof navigation.addEventListener+'|'+typeof navigation.navigate+'|'+typeof navigation.currentEntry.url+'|'+(navigation instanceof EventTarget)+'|'+(navigation instanceof Navigation)"
        ),
        "function|function|string|true|true"
    );
    // `function navigation()`/`var navigation` sloppy : la prop
    // non-inscriptible survit (comportement Chrome — l'attribut
    // window n'est pas écrasé).
    ev(&mut e, "function navigation(){return 7}; var navigation");
    assert_eq!(ev(&mut e, "typeof navigation.addEventListener"), "function");
    // Rail `__et` : enregistrement + dispatch manuel + retrait.
    assert_eq!(
        ev(
            &mut e,
            "var n=0;var cb=function(){n++};\
             navigation.addEventListener('navigate',cb);\
             navigation.dispatchEvent({type:'navigate'});\
             navigation.removeEventListener('navigate',cb);\
             navigation.dispatchEvent({type:'navigate'});\
             n"
        ),
        "1"
    );
    // `navigate()`/`back()` → `NavigationResult` promesses
    // résolues (hook embedder absent — résolution honnête).
    assert_eq!(
        ev(
            &mut e,
            "var r=navigation.navigate('/x');\
             typeof r.committed.then+'|'+typeof r.finished.then+'|'+\
             (typeof navigation.back().committed.then)+'|'+navigation.canGoForward+'|'+navigation.transition"
        ),
        "function|function|function|false|null"
    );
    // `entries()` = entrée courante, `getState()` sans état posé →
    // `undefined` ; `updateCurrentEntry` partage `history.state`.
    assert_eq!(
        ev(
            &mut e,
            "var es=navigation.entries();\
             es.length+'|'+(es[0]===navigation.currentEntry===false)+'|'+es[0].sameDocument+'|'+typeof es[0].getState+'|'+(es[0].getState()===undefined)"
        ),
        "1|true|true|function|true"
    );
}

#[test]
fn observer_protos_survive_global_wrappers_and_grafts() {
    let mut e = engine();
    // Surface WebIDL : méthodes sur le proto (gog vol 149 /
    // leclerc vol 141 — les agents qui greffent ou stubs
    // lisent `X.prototype.observe` ; un proto nu rendait
    // `undefined` → wrappers nés sans `observe`).
    assert_eq!(
        ev(
            &mut e,
            "typeof IntersectionObserver.prototype.observe+'|'+\
             typeof ResizeObserver.prototype.disconnect+'|'+\
             typeof MutationObserver.prototype.takeRecords+'|'+\
             (IntersectionObserver.prototype.constructor===IntersectionObserver)"
        ),
        "function|function|function|true"
    );
    // `instanceof` via le proto du ctor.
    assert_eq!(
        ev(
            &mut e,
            "var io=new IntersectionObserver(function(){});\
             var ro=new ResizeObserver(function(){});\
             io instanceof IntersectionObserver && ro instanceof ResizeObserver"
        ),
        "true"
    );
    // Global remplacé par un wrapper agent : l'instance réelle
    // garde le proto capturé à l'installation — pas le proto nu
    // du wrapper (le global n'est pas relu au `new`).
    assert_eq!(
        ev(
            &mut e,
            "var OO=window.MutationObserver;\
             window.MutationObserver=function(c){this.inn=new OO(c)};\
             var w=new MutationObserver(function(){});\
             typeof w.inn.observe"
        ),
        "function"
    );
    assert_eq!(
        ev(
            &mut e,
            "var OI=window.IntersectionObserver;\
             window.IntersectionObserver=function(c){return new OI(c)};\
             var w3=new IntersectionObserver(function(){});\
             typeof w3.observe+'|'+typeof w3.disconnect"
        ),
        "function|function"
    );
    // Greffe scriptée sur le proto VUE À L'APPEL : le rail
    // `invoke` cède devant le masque (`host_method_shadowed`) —
    // `io.observe()` invoque le wrapper de l'agent ; sur `?.`
    // pareil. Sans greffe le rail reste le chemin.
    assert_eq!(
        ev(
            &mut e,
            "var __c=0;var __d=0;\
             OI.prototype.observe=function(){__c++};\
             OO.prototype.observe=function(){__d++};\
             var io2=new OI(function(){});io2.observe(document.body);\
             var mo2=new OO(function(){});mo2?.observe(document.body,{childList:true});\
             __c+'|'+__d"
        ),
        "1|1"
    );
    // Rail intact : cibles Element/Document, ops complètes.
    assert_eq!(
        ev(
            &mut e,
            "var io3=new OI(function(){});\
             io3.observe(document.body);io3.observe(document);\
             io3.unobserve(document.body);io3.disconnect();\
             Array.isArray(io3.takeRecords())"
        ),
        "true"
    );
}

#[test]
fn navigation_timing_entry_is_honest_and_complete() {
    let mut e = engine();
    // techcrunch vol 188 (WP-interactivity) : `[0].domContentLoadedEventStart`
    // tombait sur `[]` → `of Undefined` — l'entrée navigation existe
    // désormais, bornes honnêtes à 0 (relatives à timeOrigin).
    assert_eq!(
        ev(
            &mut e,
            "var n=performance.getEntriesByType('navigation')[0];\
             [typeof n.domContentLoadedEventStart,typeof n.domInteractive,\
              typeof n.loadEventEnd,n.entryType,n.type,n.name===location.href,\
              n.responseStatus,n.redirectCount,n.startTime,n.duration,\
              typeof n.toJSON,n.toJSON().name===n.name].join('|')"
        ),
        "number|number|number|navigation|navigate|true|200|0|0|0|function|true"
    );
    // `getEntries`/`getEntriesByName` la couvrent aussi.
    assert_eq!(
        ev(
            &mut e,
            "performance.getEntries().some(function(x){return x.entryType==='navigation'})+'|'+\
             performance.getEntriesByName(location.href,'navigation').length+'|'+\
             (JSON.stringify(n).indexOf('domContentLoadedEventStart')>=0)+'|'+\
             performance.getEntriesByType('resource').length"
        ),
        "true|1|true|0"
    );
}

#[test]
fn tcfapi_honest_cmp_less_stub() {
    let mut e = engine();
    // all.rugby vol 183 (Sourcepoint banner.js) : `__tcfapi`
    // absent → `is not a function (on Object)`. Stub TCF v2
    // honnête : `ping` = descripteur CMP-less, commandes data
    // → `(null, false)` pour armer les fallbacks.
    assert_eq!(
        ev(
            &mut e,
            "var out=''; __tcfapi('ping',2,function(r,ok){\
             out=[typeof __tcfapi,ok,r.gdprApplies,r.cmpLoaded,r.cmpStatus,\
             typeof r.apiVersion].join('|')});\
             __tcfapi('getTCData',2,function(d,ok2){out+='|'+d+'/'+ok2});\
             __tcfapi('addEventListener',2,function(d,ok3){out+='|'+ok3}); out"
        ),
        "function|true|false|false|stub|string|null/false|false"
    );
}

#[test]
fn apply_spreads_array_like() {
    let mut e = engine();
    // `f.apply(this, arrayLike)` — CreateListFromArrayLike :
    // `length` + `[[Get]]` par indice, pas seulement les vrais
    // Arrays (Zone-lite `T.apply(this, arguments)` — ryanair
    // vol 215 : `MutationObserver: callback requis`).
    assert_eq!(
        ev(
            &mut e,
            r"function f(){ return arguments.length; }
               f.apply(null, {length:3, 0:'a', 1:'b', 2:'c'})"
        ),
        "3"
    );
    // `arguments` réel passé à une autre fonction.
    assert_eq!(
        ev(
            &mut e,
            r"function f(){ return arguments.length; }
               function g(){ return f.apply(null, arguments); }
               g(1,2)"
        ),
        "2"
    );
    // null/undefined → aucun arg (spec).
    assert_eq!(ev(&mut e, "f.apply(null, null)"), "0");
    assert_eq!(ev(&mut e, "f.apply(null, undefined)"), "0");
    // non-objet → TypeError (spec).
    let err = e
        .eval("try { f.apply(null, 5); 'no' } catch(x) { 'err' }")
        .unwrap();
    assert_eq!(e.describe(err), "err");
    // length getter + getter d'indice traversés.
    assert_eq!(
        ev(
            &mut e,
            r"var al = { get length(){ return 1; }, get 0(){ return 7; } };
               function g2(){ return arguments[0]; }
               g2.apply(null, al)"
        ),
        "7"
    );
}

#[test]
fn zone_style_native_apply_reaches_mutobs() {
    let mut e = engine();
    // Rail Zone-lite exact : méthode patchée →
    // `orig.apply(this, arguments)` — l'original natif reçoit
    // le callback (ryanair vol 215 : `callback requis`).
    assert_eq!(
        ev(
            &mut e,
            r"var out='?'; try {
               var m = MutationObserver.prototype.constructor.call(
                 MutationObserver, function(){});
               out = typeof m.observe;
             } catch(x) { out='ERR:'+x; } out"
        ),
        "function"
    );
    // Via le wrapper exact `T.apply(this, Fe(arguments))`.
    assert_eq!(
        ev(
            &mut e,
            r"var out='?'; try {
               var orig = MutationObserver.prototype.constructor;
               var wrapped = function(){ return orig.apply(this, arguments); };
               var w = wrapped(function(){});
               out = typeof w.observe;
             } catch(x) { out='ERR:'+x; } out"
        ),
        "function"
    );
}

#[test]
fn link_rel_list_supports_modulepreload() {
    let mut e = engine();
    // `link.relList` (§B-14 vols 141/216 philharmonie —
    // addtoany `link.relList.supports('modulepreload')`) —
    // DOMTokenList reflétant `rel`, vivant sur l'attribut.
    assert_eq!(
        ev(
            &mut e,
            r"var l=document.createElement('link');
               l.setAttribute('rel','modulepreload foo');
               var r=l.relList;
               [r.length, r.item(0), r.contains('foo'), r.supports('modulepreload')].join('|')"
        ),
        "2|modulepreload|true|true"
    );
    // `supports` = validation de jeton (rail partagé
    // classList — `modulepreload` jeton sain → true, chaîne
    // vide/espaces → false — ce que le feature-detect lit).
    assert_eq!(
        ev(
            &mut e,
            "document.createElement('link').relList.supports('modulepreload')"
        ),
        "true"
    );
    assert_eq!(
        ev(
            &mut e,
            "document.createElement('link').relList.supports('a b')"
        ),
        "false"
    );
    // Mutations reflétées dans l'attribut (vivant).
    assert_eq!(
        ev(
            &mut e,
            r"var l2=document.createElement('link');
               l2.relList.add('stylesheet'); l2.relList.toggle('x');
               l2.getAttribute('rel')+'/'+l2.relList.value"
        ),
        "stylesheet x/stylesheet x"
    );
    // Spec : tags sans relList → undefined.
    assert_eq!(
        ev(&mut e, "typeof document.createElement('div').relList"),
        "undefined"
    );
    // Ancres/form ont bien relList.
    assert_eq!(
        ev(
            &mut e,
            r"var a=document.createElement('a'); a.setAttribute('rel','noreferrer');
               var f=document.createElement('form'); f.setAttribute('rel','');
               a.relList.contains('noreferrer')+'/'+typeof f.relList"
        ),
        "true/object"
    );
}

#[test]
fn reflect_get_apply_inside_proxy_trap() {
    let mut e = engine();
    // Rail prebid fun-hooks (footmercato vol 183 — `install of
    // Undefined`) : trap `get` fait `Reflect.get.apply(Reflect,
    // arguments)` — dépend de l'étalement array-like (JS-C217).
    assert_eq!(
        ev(
            &mut e,
            r"var tgt = { after: 'h1', name: 'n' };
               var p = new Proxy(tgt, { get: function(r, k) {
                 return Reflect.get.apply(Reflect, arguments);
               }});
               p.after + '|' + p.name"
        ),
        "h1|n"
    );
    // Avec fallback `||` (rail réel : `v[e] || Reflect.get.apply`).
    assert_eq!(
        ev(
            &mut e,
            r"var v = { special: 'V' };
               var p2 = new Proxy({ after: 'h2' }, { get: function(r, k) {
                 return v[k] || Reflect.get.apply(Reflect, arguments);
               }});
               p2.special + '|' + p2.after"
        ),
        "V|h2"
    );
}

/// §B-14 vol 331 (GRC) — `SVGGeometryElement.getTotalLength`/
/// `getPointAtLength` : géométrie réelle calculée, jamais de
/// stub. path/ligne/rect/cercle/ellipse/poly couverts.
#[test]
fn svg_geometry_total_length_and_point_at() {
    let mut e = Engine::new();
    e.install_dom(&Rc::new(RefCell::new(Dom::from_html(
        r#"<html><body>
<svg>
<path id="p" d="M0,0 L30,0 L30,40"/>
<line id="l" x1="0" y1="0" x2="3" y2="4"/>
<rect id="r" x="0" y="0" width="10" height="5"/>
<circle id="c" r="10"/>
<polyline id="pl" points="0,0 10,0 10,10"/>
<polygon id="pg" points="0,0 10,0 10,10"/>
</svg></body></html>"#,
    ))));
    // path : 30+40 = 70
    assert_eq!(
        ev(&mut e, "document.getElementById('p').getTotalLength()"),
        "70"
    );
    // ligne : hypot(3,4) = 5
    assert_eq!(
        ev(&mut e, "document.getElementById('l').getTotalLength()"),
        "5"
    );
    // rect : 2*(10+5) = 30
    assert_eq!(
        ev(&mut e, "document.getElementById('r').getTotalLength()"),
        "30"
    );
    // cercle : 2π·10 ≈ 62.83
    assert_eq!(
        ev(
            &mut e,
            "document.getElementById('c').getTotalLength().toFixed(1)"
        ),
        "62.8"
    );
    // polyline ouverte : 10+10 = 20 ; polygon fermée :
    // 10+10+hypoténuse de fermeture √200 ≈ 34.14.
    assert_eq!(
        ev(&mut e, "document.getElementById('pl').getTotalLength()"),
        "20"
    );
    assert_eq!(
        ev(
            &mut e,
            "document.getElementById('pg').getTotalLength().toFixed(2)"
        ),
        "34.14"
    );
    // getPointAtLength : mi-chemin du path (35 sur 70) = (30,5)
    assert_eq!(
        ev(
            &mut e,
            "var q=document.getElementById('p').getPointAtLength(35);\
             q.x+':'+q.y+':'+q.z+':'+q.w"
        ),
        "30:5:0:1"
    );
    // clamp hors bornes → extrémités
    assert_eq!(
        ev(
            &mut e,
            "var q=document.getElementById('p').getPointAtLength(999);\
             q.x+':'+q.y"
        ),
        "30:40"
    );
    // tag non-géométrie → undefined (pas de méthode fantôme)
    assert_eq!(
        ev(&mut e, "typeof document.body.getTotalLength"),
        "undefined"
    );
}

/// §B-14 vol 323 — `navigator.serviceWorker` : surface
/// `ServiceWorkerContainer` honnête — `register` rejette
/// `NotSupportedError` (jamais de faux succès), `ready`
/// pending spec, `controller` null, `getRegistrations` → [].
#[test]
fn navigator_service_worker_honest_stub() {
    let mut e = engine();
    assert_eq!(ev(&mut e, "typeof navigator.serviceWorker"), "object");
    assert_eq!(
        ev(
            &mut e,
            "typeof navigator.serviceWorker.register + ':' \
             + navigator.serviceWorker.controller"
        ),
        "function:null"
    );
    // register() → promesse rejetée NotSupportedError
    assert_eq!(
        ev(
            &mut e,
            "globalThis.__r='p';\
             navigator.serviceWorker.register('/sw.js')\
               .then(()=>{__r='resolved'})\
               .catch(x=>{__r=x.name});\
             __r"
        ),
        "p"
    );
    let _ = e.eval("0"); // drain microtasks
    assert_eq!(ev(&mut e, "globalThis.__r"), "NotSupportedError");
    // getRegistration → undefined résolu ; getRegistrations → []
    assert_eq!(
        ev(
            &mut e,
            "globalThis.__r='p';\
             navigator.serviceWorker.getRegistrations()\
               .then(v=>{__r='len:'+v.length});__r"
        ),
        "p"
    );
    let _ = e.eval("0");
    assert_eq!(ev(&mut e, "globalThis.__r"), "len:0");
}

/// §B-14 ×4 (edf/lcl/kleinezeitung/hvg vols 329+) —
/// `visualViewport.addEventListener` : rail `EventTarget`
/// greffé, listeners stockés sur `__et`.
#[test]
fn visual_viewport_event_target() {
    let mut e = engine();
    assert_eq!(
        ev(
            &mut e,
            "typeof visualViewport.addEventListener + ':' \
             + typeof visualViewport.removeEventListener + ':' \
             + typeof visualViewport.dispatchEvent"
        ),
        "function:function:function"
    );
    // add/remove/dispatch vivent sans jeter
    assert_eq!(
        ev(
            &mut e,
            "var hit=0,f=function(){hit++};\
             visualViewport.addEventListener('resize',f);\
             visualViewport.dispatchEvent({type:'resize'});\
             visualViewport.removeEventListener('resize',f);\
             visualViewport.dispatchEvent({type:'resize'});hit"
        ),
        "1"
    );
}

/// §B-14 vol 343 (kleinezeitung Nuxt) — `Performance` et
/// `ScreenOrientation` héritent `EventTarget` : rail `__et`
/// greffé sur les deux.
#[test]
fn performance_and_orientation_event_target() {
    let mut e = engine();
    assert_eq!(
        ev(
            &mut e,
            "typeof performance.addEventListener + ':' \
             + typeof performance.removeEventListener + ':' \
             + typeof performance.dispatchEvent"
        ),
        "function:function:function"
    );
    // listeners stockés + tirables sur performance
    assert_eq!(
        ev(
            &mut e,
            "var hit=0,f=function(){hit++};\
             performance.addEventListener('resourcetimingbufferfull',f);\
             performance.dispatchEvent({type:'resourcetimingbufferfull'});hit"
        ),
        "1"
    );
    // screen.orientation — ScreenOrientation + ET rail
    assert_eq!(
        ev(
            &mut e,
            "typeof screen.orientation + ':' \
             + typeof screen.orientation.addEventListener + ':' \
             + screen.orientation.angle + ':' \
             + (screen.orientation.type.indexOf('landscape')>=0 \
                || screen.orientation.type.indexOf('portrait')>=0)"
        ),
        "object:function:0:true"
    );
    // lock() → promesse rejetée NotSupportedError (honnête)
    assert_eq!(
        ev(
            &mut e,
            "globalThis.__r='p';\
             screen.orientation.lock('landscape')\
               .then(()=>{__r='resolved'})\
               .catch(x=>{__r=x.name});__r"
        ),
        "p"
    );
    let _ = e.eval("0");
    assert_eq!(ev(&mut e, "globalThis.__r"), "NotSupportedError");
}

/// §B-14 vol 343 (kleinezeitung) — le `TypeError` `on Object`
/// nu ne nommait pas le receveur : les propres clés l'identifient
/// (event-bus/objet littéral) — même rail que `reçu=` d'importNode.
#[test]
fn missing_method_on_bare_object_names_own_keys() {
    let mut e = Engine::new();
    // Objet event-bus à la Nuxt : clés propres exposées.
    assert_eq!(
        ev(
            &mut e,
            "var bus={_events:{},on:function(){},emit:function(){}};\
             try{bus.addEventListener('x',function(){});'no'}\
             catch(x){x.message.indexOf('_events')>=0?'keys':'?'}"
        ),
        "keys"
    );
    // `@@toStringTag` nommé prime les clés.
    assert_eq!(
        ev(
            &mut e,
            "var o={};o[Symbol.toStringTag]='EvtBus';\
             try{o.missing();'no'}\
             catch(x){x.message.indexOf('EvtBus')>=0?'tag':'?'}"
        ),
        "tag"
    );
    // Objet vide → desc « Object » inchangée (pas de bruit).
    assert_eq!(
        ev(
            &mut e,
            "try{({}).missing();'no'}\
             catch(x){x.message.indexOf('(on Object)')>=0?'plain':'?'}"
        ),
        "plain"
    );
}

/// §B-14 vol 344 (kleinezeitung) — le receveur nommé par JS-C228
/// était `cookieStore` (`{get,getAll,set,delete}` nu) :
/// `CookieStore` hérite `EventTarget` — `addEventListener(
/// 'change', cb)` est le pattern Nuxt qui cassait entry.js.
#[test]
fn cookie_store_event_target() {
    let mut e = engine();
    assert_eq!(
        ev(
            &mut e,
            "typeof cookieStore.addEventListener + ':' \
             + typeof cookieStore.removeEventListener + ':' \
             + typeof cookieStore.dispatchEvent + ':' \
             + ({}).toString.call(cookieStore)"
        ),
        "function:function:function:[object CookieStore]"
    );
    // listener 'change' stocké + tirable sur cookieStore.
    assert_eq!(
        ev(
            &mut e,
            "var hit=0,f=function(){hit++};\
             cookieStore.addEventListener('change',f);\
             cookieStore.dispatchEvent({type:'change'});hit"
        ),
        "1"
    );
    // jar vide honnête conservé.
    assert_eq!(
        ev(
            &mut e,
            "globalThis.__r='p';\
             cookieStore.get('x').then(v=>{__r=''+v});__r"
        ),
        "p"
    );
    let _ = e.eval("0");
    assert_eq!(ev(&mut e, "globalThis.__r"), "null");
}

// JS-C233 — typage SVG réel (vivus vol 345 : `element`
// rejeté `not valid` parce que `instanceof SVGSVGElement`/
// `constructor` échouaient — les nœuds SVG étaient des
// `HTMLElement` génériques). Chaîne WebIDL complète +
// ctor global + `tagName`/`namespaceURI` spec.
#[test]
fn svg_elements_typed_with_webidl_chain() {
    let mut e = engine();
    // `<svg>` → `SVGSVGElement`, `<use>` → `SVGUseElement` —
    // instanceof transitif jusqu'à `EventTarget`, plus
    // `instanceof HTMLElement` désormais FAUX (spec).
    assert_eq!(
        ev(
            &mut e,
            "var s=document.querySelector('svg'),u=document.getElementById('u');\
             (s instanceof SVGSVGElement)+'|'\
             +(s instanceof SVGElement)+'|'\
             +(s instanceof SVGGraphicsElement)+'|'\
             +(s instanceof Element)+'|'\
             +(s instanceof Node)+'|'\
             +(s instanceof EventTarget)+'|'\
             +(u instanceof SVGUseElement)+'|'\
             +(s instanceof HTMLElement)"
        ),
        "true|true|true|true|true|true|true|false"
    );
    // ctor global installé + `constructor` tag-spécifique.
    assert_eq!(ev(&mut e, "typeof SVGGeometryElement"), "function");
    assert_eq!(
        ev(&mut e, "s.constructor.name+'|'+u.constructor.name"),
        "SVGSVGElement|SVGUseElement"
    );
    // `tagName`/`namespaceURI` spec foreign elements.
    assert_eq!(ev(&mut e, "s.tagName+'|'+u.tagName"), "svg|use");
    assert_eq!(
        ev(&mut e, "s.namespaceURI+'|'+document.body.namespaceURI"),
        "http://www.w3.org/2000/svg|http://www.w3.org/1999/xhtml"
    );
    // Feuille géométrie : `path` est `SVGGeometryElement` et
    // porte la géométrie JS-C223 réelle — le verrou historique.
    assert_eq!(
        ev(
            &mut e,
            "var p=document.createElement('path');\
             (p instanceof SVGGeometryElement)+'|'\
             +(p instanceof SVGPathElement)+'|'\
             +(typeof p.getTotalLength)"
        ),
        "true|true|function"
    );
    // `createElementNS` — même typage via le tag stocké.
    assert_eq!(
        ev(
            &mut e,
            "var r=document.createElementNS('http://www.w3.org/2000/svg','rect');\
             (r instanceof SVGRectElement)+'|'+(r instanceof SVGGeometryElement)"
        ),
        "true|true"
    );
}

/// §B-14 ×5 (vols 345 MAGHREB/ME + 346 PH/ID/TH) —
/// `ServiceWorkerContainer` hérite `EventTarget` :
/// `sw.addEventListener('controllerchange'|'message')`
/// cassait les sites SW-guardés.
#[test]
fn service_worker_container_event_target() {
    let mut e = engine();
    assert_eq!(
        ev(
            &mut e,
            "typeof navigator.serviceWorker.addEventListener + ':' \
             + typeof navigator.serviceWorker.removeEventListener + ':' \
             + typeof navigator.serviceWorker.dispatchEvent + ':' \
             + navigator.serviceWorker.oncontrollerchange + ':' \
             + navigator.serviceWorker.onmessage"
        ),
        "function:function:function:null:null"
    );
    // listeners stockés + tirables
    assert_eq!(
        ev(
            &mut e,
            "var hit=0,f=function(){hit++};\
             navigator.serviceWorker.addEventListener('controllerchange',f);\
             navigator.serviceWorker.dispatchEvent({type:'controllerchange'});hit"
        ),
        "1"
    );
}

/// §B-14 vol 367 telegram — `addEventListener on
/// Object@@SharedWorker` : le proto liait `hostMethod` mais
/// le wrapper `{port}` est un objet plain sans Host — rail
/// `__et` paresseux posé directement sur le proto.
#[test]
fn shared_worker_event_target_via_proto() {
    let mut e = engine();
    // L'instance simule la forme réelle : objet nu + proto.
    assert_eq!(
        ev(
            &mut e,
            "typeof SharedWorker.prototype.addEventListener + ':' \
             + typeof SharedWorker.prototype.removeEventListener + ':' \
             + typeof SharedWorker.prototype.dispatchEvent"
        ),
        "function:function:function"
    );
    // Proto chainé sur EventTarget — `sw` hérite du rail.
    assert_eq!(
        ev(
            &mut e,
            "var sw=Object.create(SharedWorker.prototype),hit=0,\
             f=function(){hit++};\
             sw.addEventListener('message',f);\
             sw.dispatchEvent({type:'message'});hit"
        ),
        "1"
    );
}

/// §B-66 vol instagram/whatsapp/threads — `illegal constructor`
/// (JS-C235) au `new AudioContext()` du loader Meta → cascade
/// `__d is not defined`. Ctor stub : instance proto-typée,
/// surface fingerprint (`state`/`sampleRate`/`destination`),
/// proto chaîné `EventTarget` (spec `BaseAudioContext`).
#[test]
fn audio_context_constructible_stub() {
    let mut e = engine();
    assert_eq!(
        ev(
            &mut e,
            "var c=new AudioContext();\
             (c instanceof AudioContext)+':'+(c instanceof EventTarget)+':'\
             +c.state+':'+c.sampleRate+':'+typeof c.createOscillator+':'\
             +typeof c.resume"
        ),
        "true:true:suspended:48000:function:function"
    );
    // `create*` → nœud générique chaînable, `connect` rend sa cible.
    assert_eq!(
        ev(
            &mut e,
            "var c=new AudioContext(),o=c.createOscillator(),g=c.createGain();\
             o.connect(g).connect(c.destination)===c.destination"
        ),
        "true"
    );
    // `resume()`/`close()` → promesse résolue (pas de rejet armé).
    assert_eq!(
        ev(
            &mut e,
            "var c=new AudioContext(),hit=0;\
             c.resume().then(function(){hit++});hit"
        ),
        "0"
    );
}

/// §B-14 vols 366/403/378 — interfaces `Storage`/`History` +
/// `getInstalledRelatedApps` : globals orphelins nommés —
/// `instanceof` spec, `Promise<[]>` honnête.
#[test]
fn storage_history_interfaces_and_related_apps() {
    let mut e = engine();
    e.install_storage(&Rc::new(RefCell::new(std::collections::BTreeMap::new())));
    assert_eq!(
        ev(
            &mut e,
            "typeof Storage + ':' + typeof History + ':' \
             + typeof navigator.getInstalledRelatedApps"
        ),
        "function:function:function"
    );
    // instanceof — les instances portent le proto de l'interface
    assert_eq!(
        ev(
            &mut e,
            "(localStorage instanceof Storage) + ':' \
             + (sessionStorage instanceof Storage) + ':' \
             + (history instanceof History)"
        ),
        "true:true:true"
    );
    // Ctor illégal nommé (§B-66 rail)
    assert_eq!(
        ev(&mut e, "try{new Storage()}catch(x){x.message}"),
        "illegal constructor (Storage)"
    );
    // getInstalledRelatedApps → Promise<[]>
    assert_eq!(
        ev(
            &mut e,
            "globalThis.__r='p';\
             navigator.getInstalledRelatedApps()\
               .then(v=>{__r='len:'+v.length});__r"
        ),
        "p"
    );
    let _ = e.eval("0");
    assert_eq!(ev(&mut e, "globalThis.__r"), "len:0");
}
