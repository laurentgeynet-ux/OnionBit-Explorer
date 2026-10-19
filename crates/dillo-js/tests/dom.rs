// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Tests for the standalone mini-DOM façade (JS-2): `document`,
//! elements, `innerHTML`, style/classList, events, `document.write`.

use dillo_js::{Dom, Engine};
use std::cell::RefCell;
use std::rc::Rc;

const PAGE: &str = r#"
<html><head><title>Test page</title></head><body>
<div id="main" class="box"><p>Hello <b>world</b></p></div>
<input id="inp" type="text" value="init">
<input id="cb" type="checkbox" checked>
<ul id="list"><li>a</li><li>b</li></ul>
</body></html>
"#;

fn engine_with(html: &str) -> Engine {
    let mut e = Engine::new();
    e.install_dom(&Rc::new(RefCell::new(Dom::from_html(html))));
    e
}

/// Eval `src` in a fresh engine seeded with `html`, return the
/// completion value described as a string.
fn ev(src: &str) -> String {
    ev_in(PAGE, src)
}

fn ev_in(html: &str, src: &str) -> String {
    let mut e = engine_with(html);
    match e.eval(src) {
        Ok(v) => e.describe(v),
        Err(err) => panic!("{src} -> {err}"),
    }
}

// -- document ---------------------------------------------------------

#[test]
fn get_element_by_id() {
    assert_eq!(ev("document.getElementById('main').tagName"), "DIV");
    assert_eq!(ev("document.getElementById('main').id"), "main");
    assert_eq!(ev("document.getElementById('nope')"), "null");
}

#[test]
fn node_identity_is_stable() {
    assert_eq!(
        ev("document.getElementById('main') === document.getElementById('main')"),
        "true"
    );
}

#[test]
fn document_structure_accessors() {
    assert_eq!(ev("document.title"), "Test page");
    assert_eq!(ev("document.body.tagName"), "BODY");
    assert_eq!(ev("document.documentElement.tagName"), "HTML");
    assert_eq!(ev("document.getElementsByTagName('li').length"), "2");
    assert_eq!(ev("document.getElementsByClassName('box').length"), "1");
}

#[test]
fn document_write_appends_to_body() {
    assert_eq!(
        ev("document.write('<div id=\"w\">added</div>'); document.getElementById('w').innerHTML"),
        "added"
    );
}

#[test]
fn navigator_is_fixed_and_poor() {
    assert_eq!(ev("navigator.userAgent"), "Explorer/0.1 (dillo-js)");
    // Deliberately absent: plugins, geolocation, hardware APIs…
    assert_eq!(ev("navigator.plugins === undefined"), "true");
    assert_eq!(ev("navigator.geolocation === undefined"), "true");
}

// -- elements ---------------------------------------------------------

#[test]
fn inner_html_get_set() {
    assert_eq!(
        ev("document.getElementById('main').innerHTML"),
        "<p>Hello <b>world</b></p>"
    );
    assert_eq!(
        ev("var el = document.getElementById('main');\
            el.innerHTML = '<span>x</span>';\
            el.innerHTML"),
        "<span>x</span>"
    );
}

#[test]
fn text_content() {
    assert_eq!(
        ev("document.getElementById('main').textContent"),
        "Hello world"
    );
    assert_eq!(
        ev("var el = document.getElementById('main');\
            el.textContent = 'plain';\
            el.innerHTML"),
        "plain"
    );
}

#[test]
fn class_name_and_class_list() {
    assert_eq!(
        ev("var el = document.getElementById('main');\
            el.className = 'a b';\
            el.className"),
        "a b"
    );
    assert_eq!(
        ev("var el = document.getElementById('main');\
            el.classList.add('on');\
            el.classList.contains('on') + ':' + el.getAttribute('class')"),
        "true:box on"
    );
    assert_eq!(
        ev("var el = document.getElementById('main');\
            el.classList.toggle('box');\
            el.className"),
        ""
    );
}

#[test]
fn style_property() {
    assert_eq!(
        ev("var el = document.getElementById('main');\
            el.style.backgroundColor = 'red';\
            el.getAttribute('style')"),
        "background-color: red"
    );
    assert_eq!(
        ev("var el = document.getElementById('main');\
            el.style.backgroundColor = 'red';\
            el.style.backgroundColor"),
        "red"
    );
}

#[test]
fn form_like_properties() {
    assert_eq!(ev("document.getElementById('inp').value"), "init");
    assert_eq!(
        ev("var i = document.getElementById('inp');\
            i.value = 'typed';\
            i.getAttribute('value')"),
        "typed"
    );
    // Boolean attribute → bool prop, both ways.
    assert_eq!(ev("document.getElementById('cb').checked"), "true");
    assert_eq!(
        ev("var c = document.getElementById('cb');\
            c.checked = false;\
            c.checked + ':' + c.hasAttribute('checked')"),
        "false:false"
    );
}

#[test]
fn tree_navigation() {
    assert_eq!(
        ev("var p = document.getElementsByTagName('p')[0];\
            p.parentNode.id"),
        "main"
    );
    assert_eq!(ev("document.getElementById('main').children.length"), "1");
    assert_eq!(
        ev("document.getElementById('list').children[1].textContent"),
        "b"
    );
    assert_eq!(
        ev("document.getElementById('main').firstChild.tagName"),
        "P"
    );
}

#[test]
fn query_selector_subset() {
    assert_eq!(ev("document.querySelector('#main').id"), "main");
    assert_eq!(ev("document.querySelector('.box').id"), "main");
    assert_eq!(ev("document.querySelector('p').tagName"), "P");
    assert_eq!(ev("document.querySelectorAll('li').length"), "2");
    // Complex selectors out of subset → null, not an error.
    assert_eq!(ev("document.querySelector('div > p')"), "null");
}

#[test]
fn create_and_attach() {
    assert_eq!(
        ev("var d = document.createElement('div');\
            d.appendChild(document.createTextNode('hi'));\
            document.getElementById('main').appendChild(d);\
            document.getElementById('main').children.length"),
        "2"
    );
    assert_eq!(
        ev("var d = document.createElement('div');\
            document.getElementById('main').appendChild(d);\
            document.getElementById('main').removeChild(d);\
            document.getElementById('main').children.length"),
        "1"
    );
}

// -- host semantics -----------------------------------------------------

#[test]
fn in_operator_sees_host_props() {
    assert_eq!(ev("'id' in document.getElementById('main')"), "true");
    assert_eq!(ev("'innerHTML' in document.body"), "true");
    assert_eq!(ev("'tortexplode' in document.body"), "false");
}

#[test]
fn script_expandos_work_on_host_objects() {
    // Host.set returning false → ordinary own property fallback.
    assert_eq!(
        ev("var el = document.getElementById('main');\
            el.myFlag = 42;\
            el.myFlag"),
        "42"
    );
}

#[test]
fn for_in_over_expandos() {
    assert_eq!(
        ev("var el = document.getElementById('main');\
            el.a = 1; el.b = 2;\
            var seen = '';\
            for (var k in el) { if (k === 'a' || k === 'b') seen += k; }\
            seen"),
        "ab"
    );
}

#[test]
fn typeof_and_class_tag() {
    assert_eq!(ev("typeof document"), "object");
    assert_eq!(
        ev("Object.prototype.toString.call(document.getElementById('main'))"),
        "[object HTMLElement]"
    );
    assert_eq!(
        ev("Object.prototype.toString.call(document)"),
        "[object HTMLDocument]"
    );
}

// -- events -------------------------------------------------------------

#[test]
fn add_event_listener_and_click() {
    assert_eq!(
        ev("var b = document.getElementById('main');\
            var hit = '';\
            b.addEventListener('click', function (ev) { hit = ev.type; });\
            b.click();\
            hit"),
        "click"
    );
}

#[test]
fn onclick_property_and_event_target() {
    assert_eq!(
        ev("var b = document.getElementById('main');\
            var same = false;\
            b.onclick = function (ev) { same = (ev.target === b); };\
            b.click();\
            same"),
        "true"
    );
}

#[test]
fn remove_event_listener() {
    assert_eq!(
        ev("var b = document.getElementById('main');\
            var n = 0;\
            var f = function () { n += 1; };\
            b.addEventListener('click', f);\
            b.click();\
            b.removeEventListener('click', f);\
            b.click();\
            n"),
        "1"
    );
}

// -- events: propagation (JS-3) ----------------------------------------------

#[test]
fn event_bubbles_to_parent() {
    // li (inside ul#list) → #list → body: bubbling walks ancestors.
    assert_eq!(
        ev("var li = document.getElementsByTagName('li')[0];\
            var order = '';\
            document.getElementById('list').addEventListener('click', function () { order += 'L'; });\
            document.body.addEventListener('click', function () { order += 'B'; });\
            li.click();\
            order"),
        "LB"
    );
}

#[test]
fn event_capture_runs_before_bubble() {
    assert_eq!(
        ev("var li = document.getElementsByTagName('li')[0];\
            var list = document.getElementById('list');\
            var order = '';\
            li.addEventListener('click', function () { order += 'b'; });\
            list.addEventListener('click', function () { order += 'c'; }, true);\
            list.addEventListener('click', function () { order += 'B'; });\
            li.click();\
            order"),
        "cbB"
    );
}

#[test]
fn current_target_and_target() {
    assert_eq!(
        ev("var li = document.getElementsByTagName('li')[0];\
            var seen = '';\
            document.getElementById('list').addEventListener('click', function (ev) {\
                seen = ev.target.tagName + '/' + ev.currentTarget.id; });\
            li.click();\
            seen"),
        "LI/list"
    );
}

#[test]
fn stop_propagation_halts_bubble() {
    assert_eq!(
        ev("var li = document.getElementsByTagName('li')[0];\
            var order = '';\
            li.addEventListener('click', function (ev) {\
                order += 'L'; ev.stopPropagation(); });\
            document.getElementById('list').addEventListener('click', function () { order += 'P'; });\
            li.click();\
            order"),
        "L"
    );
}

#[test]
fn dispatch_event_with_literal() {
    assert_eq!(
        ev("var b = document.getElementById('main');\
            var got = '';\
            b.addEventListener('ping', function (ev) { got = ev.type; });\
            b.dispatchEvent({ type: 'ping', bubbles: false });\
            got"),
        "ping"
    );
}

#[test]
fn dispatch_event_with_create_event() {
    assert_eq!(
        ev("var b = document.getElementById('main');\
            var got = '';\
            b.addEventListener('custom', function (ev) { got = ev.type + ':' + ev.bubbles; });\
            var ev = document.createEvent('HTMLEvents');\
            ev.initEvent('custom', true, true);\
            b.dispatchEvent(ev);\
            got"),
        "custom:true"
    );
}

#[test]
fn prevent_default_returned_by_dispatch() {
    assert_eq!(
        ev("var b = document.getElementById('main');\
            b.addEventListener('x', function (ev) { ev.preventDefault(); });\
            var ev = document.createEvent('HTMLEvents');\
            ev.initEvent('x', false, true);\
            b.dispatchEvent(ev) + ':' + ev.defaultPrevented"),
        "false:true"
    );
}

// -- timers (JS-3) -----------------------------------------------------------

#[test]
fn set_timeout_runs_on_pump() {
    let mut e = engine_with("");
    e.install_timers();
    e.eval("var done = ''; setTimeout(function () { done = 'fired'; }, 0);")
        .expect("eval");
    // Not yet: timers don't run synchronously.
    assert_eq!(
        e.eval("done").map(|v| e.describe(v)).unwrap_or_default(),
        ""
    );
    assert_eq!(e.run_due_timers().unwrap_or(0), 1);
    assert_eq!(
        e.eval("done").map(|v| e.describe(v)).unwrap_or_default(),
        "fired"
    );
}

#[test]
fn set_interval_reschedules_and_clears() {
    let mut e = engine_with("");
    e.install_timers();
    e.eval(
        "var n = 0;\
            var id = setInterval(function () { n += 1; }, 0);",
    )
    .expect("eval");
    e.run_due_timers().unwrap_or(0);
    e.run_due_timers().unwrap_or(0);
    e.eval("clearInterval(id);").expect("eval");
    e.run_due_timers().unwrap_or(0);
    assert_eq!(e.eval("n").map(|v| e.describe(v)).unwrap_or_default(), "2");
}

#[test]
fn clear_timeout_cancels() {
    let mut e = engine_with("");
    e.install_timers();
    e.eval(
        "var ran = false;\
            var id = setTimeout(function () { ran = true; }, 0);\
            clearTimeout(id);",
    )
    .expect("eval");
    e.run_due_timers().unwrap_or(0);
    assert_eq!(
        e.eval("ran").map(|v| e.describe(v)).unwrap_or_default(),
        "false"
    );
}

#[test]
fn string_callback_runs_in_global_scope() {
    // `setTimeout('code')` — legacy string form evaluated at fire
    // time in global scope (indirect-eval semantics).
    let mut e = engine_with("");
    e.install_timers();
    e.eval("var hit = 0; setTimeout('hit = 42', 0);")
        .expect("eval");
    assert_eq!(e.run_due_timers().unwrap_or(0), 1);
    assert_eq!(
        e.eval("hit").map(|v| e.describe(v)).unwrap_or_default(),
        "42"
    );
}

// -- tolerance ------------------------------------------------------------

#[test]
fn malformed_markup_degrades() {
    // Unclosed tags, stray '<', stray '</x>' — must not panic.
    assert_eq!(
        ev_in(
            "<div id=a><b>bold<p>para",
            "document.getElementById('a').innerHTML"
        ),
        "<b>bold<p>para</p></b>"
    );
}

#[test]
fn attribute_entities_and_case() {
    assert_eq!(
        ev_in(
            "<DIV ID=X data-k='a&amp;b'>t</DIV>",
            "document.getElementById('X').getAttribute('data-k')"
        ),
        "a&b"
    );
}

// -- browser surface (JS-7e): cookie, storage, location, history -------

/// In-memory `CookieJar` — `set` keeps raw `k=v` pairs (attrs ignored).
struct TestJar {
    inner: RefCell<Vec<String>>,
}

impl dillo_js::CookieJar for TestJar {
    fn get(&self, _url: &str) -> String {
        self.inner.borrow().join("; ")
    }
    fn set(&self, _url: &str, cookie: &str) {
        // Keep only `k=v` before the first ';' like a real jar's
        // `document.cookie` read-back.
        let kv = cookie.split(';').next().unwrap_or("").trim().to_string();
        self.inner.borrow_mut().push(kv);
    }
}

fn engine_full(html: &str) -> (Engine, Rc<TestJar>, Rc<RefCell<Vec<String>>>) {
    let mut e = Engine::new();
    e.install_dom(&Rc::new(RefCell::new(Dom::from_html(html))));
    // Per-page localStorage map — an embedder shares it by origin.
    e.install_storage(&Rc::new(RefCell::new(std::collections::BTreeMap::new())));
    let jar = Rc::new(TestJar {
        inner: RefCell::new(Vec::new()),
    });
    let navs: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
    let navs2 = navs.clone();
    let navs3 = navs.clone();
    e.set_page_url("http://ex.org:8080/a/b?q=1#frag");
    e.set_cookie_jar(jar.clone());
    e.set_nav_hooks(
        Rc::new(move |u: &str| navs2.borrow_mut().push(u.to_string())),
        Rc::new(move |n: i32| navs3.borrow_mut().push(format!("go:{n}"))),
        3,
    );
    (e, jar, navs)
}

fn evf(e: &mut Engine, src: &str) -> String {
    match e.eval(src) {
        Ok(v) => e.describe(v),
        Err(err) => panic!("{src} -> {err}"),
    }
}

#[test]
fn document_cookie_roundtrip() {
    let (mut e, _jar, _n) = engine_full(PAGE);
    evf(
        &mut e,
        "document.cookie = 'sess=abc'; document.cookie = 'pref=dark; path=/'",
    );
    assert_eq!(evf(&mut e, "document.cookie"), "sess=abc; pref=dark");
}

#[test]
fn storage_set_get_remove() {
    let (mut e, _jar, _n) = engine_full(PAGE);
    evf(
        &mut e,
        "localStorage.setItem('k', 'v'); localStorage.alt = 'x';",
    );
    assert_eq!(
        evf(
            &mut e,
            "localStorage.getItem('k') + '|' + localStorage['alt'] + '|' + localStorage.length"
        ),
        "v|x|2"
    );
    // Named-property assignment also stores.
    assert_eq!(
        evf(&mut e, "localStorage.key(0) + ',' + localStorage.key(1)"),
        "alt,k"
    );
    evf(&mut e, "localStorage.removeItem('k');");
    assert_eq!(evf(&mut e, "localStorage.getItem('k')"), "null");
    assert_eq!(evf(&mut e, "localStorage.length"), "1");
    evf(&mut e, "localStorage.clear();");
    assert_eq!(evf(&mut e, "localStorage.length"), "0");
    // sessionStorage is a separate, page-local map.
    evf(&mut e, "sessionStorage.setItem('s', '1');");
    assert_eq!(evf(&mut e, "sessionStorage.getItem('s')"), "1");
    assert_eq!(evf(&mut e, "localStorage.getItem('s')"), "null");
}

#[test]
fn window_location_getters() {
    let (mut e, _jar, _n) = engine_full(PAGE);
    assert_eq!(
        evf(&mut e, "location.href"),
        "http://ex.org:8080/a/b?q=1#frag"
    );
    assert_eq!(evf(&mut e, "location.protocol"), "http:");
    assert_eq!(evf(&mut e, "location.host"), "ex.org:8080");
    assert_eq!(evf(&mut e, "location.hostname"), "ex.org");
    assert_eq!(evf(&mut e, "location.port"), "8080");
    assert_eq!(evf(&mut e, "location.pathname"), "/a/b");
    assert_eq!(evf(&mut e, "location.search"), "?q=1");
    assert_eq!(evf(&mut e, "location.hash"), "#frag");
    assert_eq!(evf(&mut e, "location.origin"), "http://ex.org:8080");
    // window aliases + same object.
    assert_eq!(evf(&mut e, "window.location === location"), "true");
    assert_eq!(
        evf(&mut e, "window === self && self === globalThis"),
        "true"
    );
}

#[test]
fn window_location_navigation() {
    let (mut e, _jar, navs) = engine_full(PAGE);
    evf(
        &mut e,
        "location.href = '/next'; location.assign('/a2'); location.replace('/r');",
    );
    // reload navigates to the current URL.
    evf(&mut e, "location.reload();");
    assert_eq!(
        navs.borrow().as_slice(),
        ["/next", "/a2", "/r", "http://ex.org:8080/a/b?q=1#frag"]
    );
}

#[test]
fn window_history() {
    let (mut e, _jar, navs) = engine_full(PAGE);
    assert_eq!(evf(&mut e, "history.length"), "3");
    evf(
        &mut e,
        "history.back(); history.forward(); history.go(-2); history.go(0);",
    );
    // go(0) = reload — not a relative move, ignored by the hook.
    assert_eq!(navs.borrow().as_slice(), ["go:-1", "go:1", "go:-2"]);
}

// -- JS-7f window surface ------------------------------------------------

#[test]
fn console_hook_receives_lines() {
    let lines = Rc::new(RefCell::new(Vec::<String>::new()));
    let sink = lines.clone();
    let mut e = Engine::new();
    e.set_console_hook(Rc::new(move |line: String| sink.borrow_mut().push(line)));
    e.eval("console.log('a', 1, 'b'); console.warn('w');")
        .expect("eval");
    assert_eq!(lines.borrow().as_slice(), ["a 1 b", "w"]);
}

#[test]
fn console_absent_hook_is_noop() {
    // No hook installed — `console.log` still parses and runs.
    let mut e = engine_with("");
    e.eval("console.log('x'); 'ok'").expect("eval");
}

#[test]
fn dialogs_route_to_hooks() {
    use std::cell::Cell;
    let mut e = engine_with("");
    let seen = Rc::new(RefCell::new(Vec::<String>::new()));
    let seen2 = seen.clone();
    let answered = Rc::new(Cell::new(true));
    let answered2 = answered.clone();
    e.set_dialog_hooks(
        Rc::new(move |m: String| seen2.borrow_mut().push(m)),
        Rc::new(move |_m: String| answered2.get()),
        Rc::new(move |m: String, d: String| Some(format!("{m}:{d}"))),
    );
    e.eval("alert('hi');").expect("alert");
    assert_eq!(seen.borrow().as_slice(), ["hi"]);
    assert_eq!(
        e.eval("confirm('q?')")
            .map(|v| e.describe(v))
            .unwrap_or_default(),
        "true"
    );
    assert_eq!(
        e.eval("prompt('name', 'def')")
            .map(|v| e.describe(v))
            .unwrap_or_default(),
        "name:def"
    );
}

#[test]
fn dialogs_absent_degrade() {
    // No hooks → alert no-op, confirm false, prompt null.
    assert_eq!(ev("confirm('x')"), "false");
    assert_eq!(ev("prompt('x')"), "null");
    assert_eq!(ev("alert('x'); 'ran'"), "ran");
}

#[test]
fn window_open_and_screen() {
    use std::cell::RefCell as RC;
    let opened = Rc::new(RC::new(Vec::<String>::new()));
    let o = opened.clone();
    let mut e = Engine::new();
    e.set_viewport(1280, 800);
    e.install_dom(&Rc::new(RC::new(Dom::from_html(""))));
    e.set_open_hook(Rc::new(move |u: String| o.borrow_mut().push(u)));
    e.eval("window.open('http://x/');").expect("open");
    assert_eq!(opened.borrow().as_slice(), ["http://x/"]);
    assert_eq!(
        e.eval("innerWidth")
            .map(|v| e.describe(v))
            .unwrap_or_default(),
        "1280"
    );
    assert_eq!(
        e.eval("screen.height")
            .map(|v| e.describe(v))
            .unwrap_or_default(),
        "800"
    );
    assert_eq!(
        e.eval("screen.colorDepth")
            .map(|v| e.describe(v))
            .unwrap_or_default(),
        "24"
    );
    assert_eq!(
        e.eval("devicePixelRatio")
            .map(|v| e.describe(v))
            .unwrap_or_default(),
        "1"
    );
}

// ---------------------------------------------------------------
// JS-7g — DOM traversal, selectors, mutation, collections
// ---------------------------------------------------------------

#[test]
fn traversal_props() {
    assert_eq!(
        ev("document.getElementById('list').firstChild.tagName"),
        "LI"
    );
    assert_eq!(
        ev("document.getElementById('list').lastChild.textContent"),
        "b"
    );
    assert_eq!(ev("document.getElementById('list').children.length"), "2");
    assert_eq!(ev("document.getElementById('list').childNodes.length"), "2");
    assert_eq!(
        ev("document.getElementById('list').children[0].nextSibling.textContent"),
        "b"
    );
    assert_eq!(
        ev("document.getElementById('list').children[1].previousSibling.textContent"),
        "a"
    );
    assert_eq!(
        ev("document.getElementById('main').children[0].parentNode.id"),
        "main"
    );
    // Text nodes are skipped by `children` but counted by childNodes.
    assert_eq!(
        ev_in(
            "<div id='x'>t<b>y</b></div>",
            "document.getElementById('x').childNodes.length"
        ),
        "2"
    );
    assert_eq!(
        ev_in(
            "<div id='x'>t<b>y</b></div>",
            "document.getElementById('x').children.length"
        ),
        "1"
    );
}

#[test]
fn matches_closest_query() {
    assert_eq!(ev("document.getElementById('main').matches('div')"), "true");
    assert_eq!(
        ev("document.getElementById('main').matches('.box')"),
        "true"
    );
    assert_eq!(
        ev("document.getElementById('main').matches('#main')"),
        "true"
    );
    assert_eq!(
        ev("document.getElementById('main').matches('span')"),
        "false"
    );
    assert_eq!(ev("document.querySelector('#list li').tagName"), "LI");
    assert_eq!(ev("document.querySelectorAll('li').length"), "2");
    assert_eq!(
        ev("document.querySelectorAll('#list li')[1].textContent"),
        "b"
    );
    // closest walks ancestors.
    assert_eq!(
        ev("document.getElementById('list').children[0].closest('ul').id"),
        "list"
    );
    assert_eq!(
        ev("document.getElementById('list').children[0].closest('div.nope')"),
        "null"
    );
    // Element-scoped query.
    assert_eq!(
        ev("document.getElementById('main').querySelectorAll('b').length"),
        "1"
    );
}

#[test]
fn mutation_methods() {
    assert_eq!(
        ev("var l = document.getElementById('list'); var li = document.createElement('li'); li.textContent = 'c'; l.appendChild(li); l.children.length"),
        "3"
    );
    assert_eq!(
        ev("var l = document.getElementById('list'); var li = document.createElement('li'); l.prepend(li); l.children[0] === li"),
        "true"
    );
    assert_eq!(
        ev("var l = document.getElementById('list'); var n = document.createElement('li'); l.children[0].before(n); l.children[0] === n"),
        "true"
    );
    assert_eq!(
        ev("var l = document.getElementById('list'); var n = document.createElement('li'); l.children[0].after(n); l.children[1] === n"),
        "true"
    );
    assert_eq!(
        ev("var l = document.getElementById('list'); var n = document.createElement('li'); l.children[0].replaceWith(n); l.children[0] === n"),
        "true"
    );
    assert_eq!(
        ev("var l = document.getElementById('list'); var c = l.children[0]; l.removeChild(c); l.children.length"),
        "1"
    );
    assert_eq!(
        ev("var l = document.getElementById('list'); var n = document.createElement('li'); l.replaceChild(n, l.children[1]); l.children[1] === n"),
        "true"
    );
    assert_eq!(
        ev("var l = document.getElementById('list'); l.children[0].remove(); l.children.length"),
        "1"
    );
    // insertBefore with ref + at end.
    assert_eq!(
        ev("var l = document.getElementById('list'); var n = document.createElement('li'); l.insertBefore(n, l.children[1]); l.children[1] === n"),
        "true"
    );
    // cloneNode shallow vs deep.
    assert_eq!(
        ev("var c = document.getElementById('list').cloneNode(false); c.children.length"),
        "0"
    );
    assert_eq!(
        ev("var c = document.getElementById('list').cloneNode(true); c.children.length"),
        "2"
    );
    // insertAdjacentHTML positions.
    assert_eq!(
        ev("var l = document.getElementById('list'); l.insertAdjacentHTML('beforeend', '<li>z</li>'); l.children[2].textContent"),
        "z"
    );
    assert_eq!(
        ev("var l = document.getElementById('list'); l.insertAdjacentHTML('afterbegin', '<li>z</li>'); l.children[0].textContent"),
        "z"
    );
    assert_eq!(
        ev("var m = document.getElementById('main'); m.insertAdjacentText('beforeend', 'tail'); m.textContent.indexOf('tail') >= 0"),
        "true"
    );
}

#[test]
fn collections_and_document_props() {
    assert_eq!(
        ev_in(
            "<a href='x'>1</a><a href='y'>2</a><img src='i'><form id='f'></form>",
            "document.links.length + '/' + document.images.length + '/' + document.forms.length"
        ),
        "2/1/1"
    );
    assert_eq!(
        ev_in(
            "<a href='x'>1</a><a href='y'>2</a>",
            "document.links[0].tagName"
        ),
        "A"
    );
    assert_eq!(ev("document.readyState"), "complete");
    assert_eq!(ev("document.head.tagName"), "HEAD");
    assert_eq!(ev("document.body.tagName"), "BODY");
}

#[test]
fn outer_html_and_dataset() {
    assert_eq!(
        ev("document.getElementById('list').outerHTML.indexOf('<ul')"),
        "0"
    );
    assert_eq!(
        ev_in(
            "<div id='d' data-user-id='42' data-foo='bar'></div>",
            "var d = document.getElementById('d'); d.dataset.userId + '/' + d.dataset.foo"
        ),
        "42/bar"
    );
    assert_eq!(
        ev_in(
            "<div id='d'></div>",
            "var d = document.getElementById('d'); d.dataset.newKey = 'v'; d.getAttribute('data-new-key')"
        ),
        "v"
    );
    assert_eq!(
        ev("document.body.contains(document.getElementById('list'))"),
        "true"
    );
    assert_eq!(
        ev("document.getElementById('list').contains(document.body)"),
        "false"
    );
}

// -- MutationObserver (JS-C26) -----------------------------------------
//
// Les callbacks sont livrés au checkpoint microtask (fin
// d'`eval`) — le test lit leurs effets via une sonde `eval`
// séparée.

/// `setup` puis `probe` sur le même moteur — le probe lit les
/// effets des callbacks `MutationObserver` livrés au checkpoint
/// du premier eval.
fn ev2(html: &str, setup: &str, probe: &str) -> String {
    let mut e = engine_with(html);
    match e.eval(setup) {
        Ok(_) => {}
        Err(err) => panic!("{setup} -> {err}"),
    }
    match e.eval(probe) {
        Ok(v) => e.describe(v),
        Err(err) => panic!("{probe} -> {err}"),
    }
}

#[test]
fn mutation_observer_child_list() {
    assert_eq!(
        ev2(
            PAGE,
            "var log=[];\
             var mo=new MutationObserver(function(rs,o){\
               log.push(rs[0].type+'|'+rs[0].addedNodes.length+'|'+(o===mo));\
             });\
             mo.observe(document.body,{childList:true});\
             document.body.appendChild(document.createElement('i'));",
            "log.join(',')"
        ),
        "childList|1|true"
    );
}

#[test]
fn mutation_observer_attributes_and_old_value() {
    assert_eq!(
        ev2(
            PAGE,
            "var log=[];\
             var mo=new MutationObserver(function(rs){\
               log.push(rs[0].type+'|'+rs[0].attributeName+'|'+rs[0].oldValue);\
             });\
             var el=document.getElementById('main');\
             mo.observe(el,{attributes:true,attributeOldValue:true});\
             el.setAttribute('class','new');",
            "log.join(',')"
        ),
        "attributes|class|box"
    );
}

#[test]
fn mutation_observer_attribute_filter() {
    assert_eq!(
        ev2(
            PAGE,
            "var n=0;\
             var mo=new MutationObserver(function(){n++});\
             var el=document.getElementById('main');\
             mo.observe(el,{attributes:true,attributeFilter:['class']});\
             el.setAttribute('id','z');\
             el.setAttribute('class','q');\
             el.removeAttribute('title');",
            "n"
        ),
        "1"
    );
}

#[test]
fn mutation_observer_subtree() {
    assert_eq!(
        ev2(
            PAGE,
            "var n=0;\
             var mo=new MutationObserver(function(){n++});\
             mo.observe(document.body,{childList:true,subtree:true});\
             document.getElementById('list').appendChild(document.createElement('li'));\
             document.getElementById('inp').setAttribute('value','x');",
            "n"
        ),
        "1"
    );
}

#[test]
fn mutation_observer_batches_once_per_checkpoint() {
    assert_eq!(
        ev2(
            PAGE,
            "var calls=0,total=0;\
             var mo=new MutationObserver(function(rs){calls++;total+=rs.length});\
             mo.observe(document.body,{childList:true});\
             document.body.appendChild(document.createElement('i'));\
             document.body.appendChild(document.createElement('b'));\
             document.body.appendChild(document.createElement('u'));",
            "calls+'/'+total"
        ),
        "1/3"
    );
}

#[test]
fn mutation_observer_disconnect_stops_delivery() {
    assert_eq!(
        ev2(
            PAGE,
            "var n=0;\
             var mo=new MutationObserver(function(){n++});\
             mo.observe(document.body,{childList:true});\
             document.body.appendChild(document.createElement('i'));\
             mo.disconnect();\
             document.body.appendChild(document.createElement('b'));",
            "n"
        ),
        // disconnect purge la file : le record déjà produit
        // n'est pas livré non plus (spec).
        "0"
    );
}

#[test]
fn mutation_observer_take_records() {
    assert_eq!(
        ev2(
            PAGE,
            "var called=0,len=0;\
             var mo=new MutationObserver(function(){called++});\
             mo.observe(document.body,{childList:true});\
             document.body.appendChild(document.createElement('i'));\
             len=mo.takeRecords().length;\
             document.body.appendChild(document.createElement('b'));",
            "len+'/'+called"
        ),
        // takeRecords vide la file — le record suivant est
        // livré normalement au checkpoint.
        "1/1"
    );
}

#[test]
fn mutation_observer_character_data() {
    assert_eq!(
        ev2(
            PAGE,
            "var log='';\
             var tn=document.createTextNode('abc');\
             var mo=new MutationObserver(function(rs){\
               log=rs[0].type+'|'+rs[0].oldValue;\
             });\
             mo.observe(tn,{characterData:true,characterDataOldValue:true});\
             tn.data='xyz';",
            "log"
        ),
        "characterData|abc"
    );
}

#[test]
fn mutation_observer_callback_can_mutate_again() {
    assert_eq!(
        ev2(
            PAGE,
            "var n=0;\
             var mo=new MutationObserver(function(){\
               if(n++<2){document.body.appendChild(document.createElement('i'))}\
             });\
             mo.observe(document.body,{childList:true});\
             document.body.appendChild(document.createElement('b'));",
            "n"
        ),
        // 1re livraison → le callback remute → nouvelle file →
        // 2e livraison au même checkpoint (boucle bornée).
        "3"
    );
}

#[test]
fn mutation_observer_survives_gc() {
    // Le callback n'est référencé que par `mutobs.cbs` — un
    // GC entre observe et mutation ne doit pas le balayer.
    let mut e = engine_with(PAGE);
    e.eval(
        "var log=[];\
         new MutationObserver(function(){log.push(1)})\
           .observe(document.body,{childList:true});",
    )
    .unwrap_or_else(|err| panic!("setup -> {err}"));
    e.collect();
    e.eval("document.body.appendChild(document.createElement('i'));")
        .unwrap_or_else(|err| panic!("mutate -> {err}"));
    match e.eval("log.join(',')") {
        Ok(v) => assert_eq!(e.describe(v), "1"),
        Err(err) => panic!("probe -> {err}"),
    }
}

// -- constraint validation (étape 54) -----------------------------------

const FORM_PAGE: &str = r#"
<html><body>
<form id="f" action="/go">
<input id="q" name="q" required>
<input id="n" type="number" name="n" min="1" max="10" value="42">
<input id="p" name="p" pattern="[0-9]+" value="abc">
<input id="ok" name="ok" value="fine">
</form>
</body></html>
"#;

#[test]
fn validity_flags_on_invalid_controls() {
    assert_eq!(
        ev_in(
            FORM_PAGE,
            "document.getElementById('q').validity.valueMissing"
        ),
        "true"
    );
    assert_eq!(
        ev_in(
            FORM_PAGE,
            "document.getElementById('n').validity.rangeOverflow"
        ),
        "true"
    );
    assert_eq!(
        ev_in(
            FORM_PAGE,
            "document.getElementById('p').validity.patternMismatch"
        ),
        "true"
    );
    assert_eq!(
        ev_in(FORM_PAGE, "document.getElementById('ok').validity.valid"),
        "true"
    );
}

#[test]
fn check_validity_and_will_validate() {
    assert_eq!(
        ev_in(FORM_PAGE, "document.getElementById('f').checkValidity()"),
        "false"
    );
    assert_eq!(
        ev_in(FORM_PAGE, "document.getElementById('ok').checkValidity()"),
        "true"
    );
    assert_eq!(
        ev_in(FORM_PAGE, "document.getElementById('q').willValidate"),
        "true"
    );
    // `disabled` ne valide jamais.
    assert_eq!(
        ev_in(
            FORM_PAGE,
            "document.getElementById('q').disabled = true;\
             document.getElementById('q').willValidate"
        ),
        "false"
    );
}

#[test]
fn custom_validity_and_message() {
    assert_eq!(
        ev_in(
            FORM_PAGE,
            "var i = document.getElementById('ok');\
             i.setCustomValidity('nope');\
             i.validationMessage"
        ),
        "nope"
    );
    assert_eq!(
        ev_in(
            FORM_PAGE,
            "var i = document.getElementById('ok');\
             i.setCustomValidity('nope');\
             i.validity.customError"
        ),
        "true"
    );
    assert_eq!(
        ev_in(
            FORM_PAGE,
            "var i = document.getElementById('ok');\
             i.setCustomValidity('nope'); i.setCustomValidity('');\
             i.validity.valid"
        ),
        "true"
    );
    // L'état transite par l'attribut interne (canal miroir→moteur).
    assert_eq!(
        ev_in(
            FORM_PAGE,
            "var i = document.getElementById('ok');\
             i.setCustomValidity('nope');\
             i.getAttribute('data-explorer-cv')"
        ),
        "nope"
    );
}

#[test]
fn invalid_event_fires_on_failed_check() {
    assert_eq!(
        ev_in(
            FORM_PAGE,
            "var hits = 0;\
             document.getElementById('q').addEventListener('invalid', function(){hits++});\
             document.getElementById('f').checkValidity();\
             hits"
        ),
        "1"
    );
}

#[test]
fn report_validity_returns_bool() {
    assert_eq!(
        ev_in(FORM_PAGE, "document.getElementById('f').reportValidity()"),
        "false"
    );
    assert_eq!(
        ev_in(FORM_PAGE, "document.getElementById('ok').reportValidity()"),
        "true"
    );
}

#[test]
fn minlength_maxlength_step_and_radio_group() {
    const P: &str = r#"<html><body><form id="f">
    <input id="s" minlength="3" value="xy">
    <input id="l" maxlength="2" value="abcd">
    <input id="st" type="number" min="0" step="2" value="3">
    <input id="r1" type="radio" name="g" required>
    <input id="r2" type="radio" name="g">
    </form></body></html>"#;
    assert_eq!(
        ev_in(P, "document.getElementById('s').validity.tooShort"),
        "true"
    );
    assert_eq!(
        ev_in(P, "document.getElementById('l').validity.tooLong"),
        "true"
    );
    assert_eq!(
        ev_in(P, "document.getElementById('st').validity.stepMismatch"),
        "true"
    );
    // `required` radio : le groupe entier est en échec tant
    // qu'aucun n'est coché, valide dès que l'un l'est.
    assert_eq!(
        ev_in(P, "document.getElementById('r1').validity.valueMissing"),
        "true"
    );
    assert_eq!(
        ev_in(
            P,
            "document.getElementById('r2').checked = true;\
             document.getElementById('r1').validity.valid"
        ),
        "true"
    );
}

#[test]
fn form_submit_calls_hook_without_event() {
    let calls = Rc::new(RefCell::new(Vec::new()));
    let mut e = engine_with(FORM_PAGE);
    {
        let c = calls.clone();
        e.set_form_submit_hook(Rc::new(move |p: dillo_js::FormSubmit| {
            c.borrow_mut().push(p);
        }));
    }
    // `submit()` : pas d'événement `submit` (spec), appel
    // direct du hook avec les champs.
    e.eval(
        "var hits = 0;\
         document.getElementById('f').addEventListener('submit', function(){hits++});\
         document.getElementById('f').submit();",
    )
    .unwrap_or_else(|err| panic!("submit -> {err}"));
    assert_eq!(calls.borrow().len(), 1);
    assert_eq!(e.eval("hits").map(|v| e.describe(v)).unwrap(), "0");
    let p = calls.borrow()[0].clone();
    assert_eq!(p.action, "/go");
    assert!(p.fields.iter().any(|(n, _)| n == "q"));
}

#[test]
fn request_submit_fires_event_then_hook() {
    let calls = Rc::new(RefCell::new(Vec::new()));
    let mut e = engine_with(FORM_PAGE);
    {
        let c = calls.clone();
        e.set_form_submit_hook(Rc::new(move |p: dillo_js::FormSubmit| {
            c.borrow_mut().push(p);
        }));
    }
    // Formulaire invalide (required vide) → `invalid` part,
    // aucune soumission.
    e.eval(
        "var inv = 0;\
         document.getElementById('q').addEventListener('invalid', function(){inv++});\
         document.getElementById('f').requestSubmit();",
    )
    .unwrap_or_else(|err| panic!("requestSubmit -> {err}"));
    assert!(calls.borrow().is_empty());
    assert_eq!(e.eval("inv").map(|v| e.describe(v)).unwrap(), "1");
    // Formulaire rendu valide + `submit` non annulé → hook.
    e.eval(
        "var sub = 0;\
         document.getElementById('f').addEventListener('submit', function(){sub++});\
         document.getElementById('q').value = 'x';\
         document.getElementById('n').value = '5';\
         document.getElementById('p').value = '42';\
         document.getElementById('f').requestSubmit();",
    )
    .unwrap_or_else(|err| panic!("requestSubmit2 -> {err}"));
    assert_eq!(e.eval("sub").map(|v| e.describe(v)).unwrap(), "1");
    assert_eq!(calls.borrow().len(), 1);
    // `preventDefault` bloque la soumission.
    e.eval(
        "document.getElementById('f').addEventListener('submit', function(ev){ev.preventDefault()});\
         document.getElementById('f').requestSubmit();",
    )
    .unwrap_or_else(|err| panic!("requestSubmit3 -> {err}"));
    assert_eq!(calls.borrow().len(), 1);
}

/// Étape 55c — le `</tag` de fermeture rawtext est cherché
/// par scan insensible à la casse **directement** dans le
/// flux de caractères : un `to_lowercase()` sur le reste du
/// document changeait la longueur en octets des runes
/// (`ﬀ`→`ff`, `İ`→`i̇`…) et l'index ramené désalignait la
/// coupe — `</script>` raté, le contenu suivant parsé comme
/// texte hors de l'élément (JSON `<script>` peints en page,
/// wikipedia — terrain mission 6/7).
#[test]
fn rawtext_close_survives_length_shifting_runes() {
    // 'ﬀ' (U+FB00) : lowercased → "ff" — 3 octets → 2.
    let html = "<div><script>var a='ﬀx';</script><p>after</p></div>";
    let d = Dom::from_html(html);
    let s = d.serialize_document();
    assert!(
        s.contains("<p>after</p>"),
        "l'élément suivant survit intact : {s}"
    );
    // Le texte du script reste **dans** l'élément : rien de
    // brut ne fuit comme nœud texte sibling.
    assert!(s.contains("<script>var a='ﬀx';</script>"));

    // Round-trip : la re-parse ne produit aucun texte hors
    // élément (la source du script ne devient jamais un
    // `Text` visible dans le DOM resynchronisé).
    let d2 = Dom::from_html(&s);
    let s2 = d2.serialize_document();
    assert!(s2.contains("<p>after</p>"), "structure stable : {s2}");

    // Casse mélangée de la balise fermante — tolérée.
    let d3 = Dom::from_html("<div><script>var a=1;</ScRiPt><b>z</b></div>");
    assert!(d3.serialize_document().contains("<b>z</b>"));
}

/// 55c — fidélité round-trip : le contenu rawtext (`script`,
/// `style`) est émis **verbatim** (le re-parseur ne décode
/// pas d'entités en rawtext — échapper double à chaque
/// resynchronisation). `title`/`textarea` sont RCDATA et
/// gardent l'échappement décodé au re-parse.
#[test]
fn rawtext_serialization_is_verbatim() {
    let html = "<div><script>var y=1&lt;2&amp;&amp;3</script><title>A&amp;B</title></div>";
    let d = Dom::from_html(html);
    let s = d.serialize_document();
    // La source du script reste brute — pas de `&amp;lt;`.
    assert!(
        s.contains("<script>var y=1&lt;2&amp;&amp;3</script>"),
        "script verbatim : {s}"
    );
    // Le titre reste échappé (RCDATA décodé au re-parse).
    assert!(s.contains("<title>A&amp;B</title>"), "title RCDATA : {s}");

    // Round-trip stable : re-serialiser ne dégrade rien.
    let s2 = Dom::from_html(&s).serialize_document();
    assert_eq!(s, s2, "round-trip stable");

    // Source contenant son propre `</script` : repli échappé
    // — la frontière tient au re-parse, rien ne fuit.
    let tricky = "<div><script>document.write('</scri'+'pt>')</script><p>x</p></div>";
    let d3 = Dom::from_html(tricky);
    let s3 = d3.serialize_document();
    let d4 = Dom::from_html(&s3);
    assert!(
        d4.serialize_document().contains("<p>x</p>"),
        "aucune fuite hors script : {s3}"
    );
}

/// Étape 55f — le chemin du sous-arbre sale : mutations
/// rapportées par `take_dirty_path` au format
/// `(tag, index)` depuis la racine ; un nœud détaché ne fixe
/// pas la cible (couvert à l'attachement par la marque du
/// parent) ; deux sous-arbres distincts escaladent à
/// l'ancêtre commun.
#[test]
fn dirty_path_tracks_mutated_subtree() {
    let mut dom = Dom::from_html(
        "<html><body><div id=\"a\"><p>x</p></div><div id=\"b\">y</div></body></html>",
    );
    dom.take_dirty();
    dom.take_dirty_path(); // purge construction (cf. install_dom)
    assert_eq!(dom.take_dirty_path(), None, "rien de sale après purge");

    let body = dom.parent(dom.by_id("b").unwrap()).unwrap();
    // Mutation d'un nœud détaché puis attachement : la cible
    // est le parent d'attachement, pas le fragment.
    let frag = dom.create_element("span");
    let text = dom.create_text("z");
    dom.append(frag, text);
    dom.append(body, frag);
    assert_eq!(
        dom.take_dirty_path(),
        Some(vec![("html".to_string(), 0), ("body".to_string(), 0)]),
        "l'attachement vise le parent, pas le fragment détaché"
    );

    // Deux sous-arbres distincts → ancêtre commun.
    let first = dom.by_id("a").unwrap();
    let second = dom.by_id("b").unwrap();
    dom.set_attr(first, "class", "m");
    dom.set_attr(second, "class", "n");
    assert_eq!(
        dom.take_dirty_path(),
        Some(vec![("html".to_string(), 0), ("body".to_string(), 0)]),
        "mutations dispersées → ancêtre commun (body)"
    );

    // Une seule cible → son propre chemin.
    let para = dom.element_children(first)[0];
    dom.set_text(dom.child_nodes(para)[0], "q");
    assert_eq!(
        dom.take_dirty_path(),
        Some(vec![
            ("html".to_string(), 0),
            ("body".to_string(), 0),
            ("div".to_string(), 0),
            ("p".to_string(), 0)
        ]),
        "mutation localisée → sous-arbre précis"
    );
}

// -- `<canvas>` 2D (JS-C28, couture R-11) ------------------------------

const PAGE_CANVAS: &str = r#"
<html><body><canvas id="c" width="100" height="80"></canvas></body></html>
"#;

#[test]
fn canvas_context_records_ops() {
    use dillo_js::CanvasOpRecord as R;
    let dom = Rc::new(RefCell::new(Dom::from_html(PAGE_CANVAS)));
    let mut e = Engine::new();
    e.install_dom(&dom);
    e.eval(
        r"const ctx = document.getElementById('c').getContext('2d');
          ctx.fillStyle = '#ff0000';
          ctx.fillRect(1, 2, 30, 40);
          ctx.globalAlpha = 0.5;
          ctx.beginPath();
          ctx.moveTo(0, 0); ctx.lineTo(10, 10); ctx.stroke();",
    )
    .expect("eval canvas");
    let ops = dom.borrow().canvas_ops();
    assert_eq!(ops.len(), 1);
    let (path, recs) = &ops[0];
    assert_eq!(path.last().map(|(t, _)| t.as_str()), Some("canvas"));
    assert_eq!(
        recs.as_slice(),
        &[
            R::FillColor {
                r: 255,
                g: 0,
                b: 0,
                a: 255
            },
            R::FillRect(1.0, 2.0, 30.0, 40.0),
            R::GlobalAlpha(0.5),
            R::BeginPath,
            R::MoveTo(0.0, 0.0),
            R::LineTo(10.0, 10.0),
            R::Stroke,
        ]
    );
}

#[test]
fn canvas_getctx_kinds_and_identity() {
    let dom = Rc::new(RefCell::new(Dom::from_html(PAGE_CANVAS)));
    let mut e = Engine::new();
    e.install_dom(&dom);
    // Spec : un seul contexte par mode ; mode inconnu → null ;
    // non-canvas → null.
    let webgl = e
        .eval("document.getElementById('c').getContext('webgl')")
        .expect("webgl");
    assert_eq!(e.describe(webgl), "null");
    let same = e
        .eval(
            "document.getElementById('c').getContext('2d') === \
             document.getElementById('c').getContext('2d')",
        )
        .expect("identity");
    assert_eq!(e.describe(same), "true");
    let not_canvas = e
        .eval("document.body.getContext('2d')")
        .expect("non-canvas");
    assert_eq!(e.describe(not_canvas), "null");
}

#[test]
fn canvas_op_marks_dirty_path() {
    // Un dessin est une mutation de rendu — le `<canvas>` est
    // le chemin sale rapporté à la synchro (relayout ciblé).
    let dom = Rc::new(RefCell::new(Dom::from_html(PAGE_CANVAS)));
    dom.borrow().take_dirty();
    dom.borrow().take_dirty_path();
    let mut e = Engine::new();
    e.install_dom(&dom);
    e.eval("document.getElementById('c').getContext('2d').fillRect(0,0,5,5)")
        .expect("fillRect");
    let path = dom
        .borrow()
        .take_dirty_path()
        .expect("une op canvas doit salir");
    assert_eq!(path.last().map(|(t, _)| t.as_str()), Some("canvas"));
}

#[test]
fn canvas_width_reset_clears_ops() {
    // `canvas.width =` efface le bitmap enregistré (spec).
    let dom = Rc::new(RefCell::new(Dom::from_html(PAGE_CANVAS)));
    let mut e = Engine::new();
    e.install_dom(&dom);
    e.eval(
        r"const c = document.getElementById('c');
          const ctx = c.getContext('2d');
          ctx.fillRect(0, 0, 9, 9);
          c.width = 200;",
    )
    .expect("resize");
    assert!(
        dom.borrow().canvas_ops().is_empty() || dom.borrow().canvas_ops()[0].1.is_empty(),
        "width= efface la op-list"
    );
    // La valeur numérique est relue depuis l'attribut.
    let w = e.eval("c.width").expect("width prop");
    assert_eq!(e.describe(w), "200");
}
