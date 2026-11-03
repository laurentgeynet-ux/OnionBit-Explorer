// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! JS-C259 — `__tcfapi` résolveur + APIs sœurs CMP (§B-4
//! bras CMP, ADR-0031, étape CMP-2).
//!
//! - `CmpPolicy::None` (défaut) → stub honnête CMP-less
//!   inchangé (JS-C213).
//! - `RejectAll`/`AcceptAll` → descripteur CMP chargée,
//!   `addEventListener` invoque le callback avec un `tcData`
//!   formé IAB v2 (`eventStatus:'tcloaded'`, `listenerId`
//!   croissant), bitfields selon la politique.
//! - Props `FIXED` : ni `window.__tcfapi = …` ni
//!   `Object.defineProperty` ni `didomiOnReady = []` ne
//!   remplacent le résolveur (résilience aux redéfinitions).
//! - Sœurs bornées : `__cmp`, `__uspapi`, `__gpp`,
//!   `__sdcmpapi`, `OneTrust`, Didomi, `_sp_`, `Osano`.
//! - Contextes enfants : la décision est engine-globale —
//!   `install_dom` du realm enfant pose le même résolveur.

use explorer_jscore::{CmpConsent, CmpPolicy, Dom, Engine};
use std::cell::RefCell;
use std::rc::Rc;

const PAGE: &str = r#"<html><body><div id="x">?</div></body></html>"#;

/// Moteur par défaut — `CmpPolicy::None` (honnête).
fn engine() -> Engine {
    let mut e = Engine::new();
    e.install_dom(&Rc::new(RefCell::new(Dom::from_html(PAGE))));
    e
}

/// Moteur sous politique résolue — `set_cmp_consent` avant
/// `install_dom` (même ordre que `Session::spawn_engine`).
fn engine_policy(policy: CmpPolicy) -> Engine {
    let mut e = Engine::new();
    e.set_cmp_consent(CmpConsent {
        policy,
        tc_string: "TC_TEST".to_string(),
        max_vendor_id: 12,
        gdpr_applies: true,
    });
    e.install_dom(&Rc::new(RefCell::new(Dom::from_html(PAGE))));
    e
}

fn ev(e: &mut Engine, src: &str) -> String {
    match e.eval(src) {
        Ok(v) => e.describe(v),
        Err(err) => panic!("{src} -> {err}"),
    }
}

// ----------------------------------------------------------------
// `CmpPolicy::None` — stub honnête JS-C213 inchangé
// ----------------------------------------------------------------

/// `None` : `ping` CMP-less, commandes data → `(null,false)`.
#[test]
fn honest_policy_keeps_cmp_less_stub() {
    let mut e = engine();
    e.eval("var p;__tcfapi('ping',2,function(d,ok){p=[ok,d.cmpStatus,d.cmpLoaded,d.gdprApplies]})")
        .unwrap();
    assert_eq!(ev(&mut e, "p.join(',')"), "true,stub,false,false");
    e.eval("var r;__tcfapi('addEventListener',2,function(d,ok){r=[d===null,ok]})")
        .unwrap();
    assert_eq!(ev(&mut e, "r.join(',')"), "true,false");
}

// ----------------------------------------------------------------
// `__tcfapi` résolveur
// ----------------------------------------------------------------

/// `ping` → descripteur CMP chargée (honnête : `cmpId 0`).
#[test]
fn reject_ping_reports_loaded_cmp() {
    let mut e = engine_policy(CmpPolicy::RejectAll);
    e.eval("var p;__tcfapi('ping',2,function(d,ok){p=[ok,d.cmpStatus,d.cmpLoaded,d.gdprApplies,d.cmpId]})")
        .unwrap();
    assert_eq!(ev(&mut e, "p.join(',')"), "true,loaded,true,true,0");
}

/// `addEventListener` → `tcData` IAB v2 complet au refus :
/// statuts, `listenerId`, bits purposes/vendors à faux.
#[test]
fn reject_add_event_listener_delivers_tcdata() {
    let mut e = engine_policy(CmpPolicy::RejectAll);
    e.eval(
        "var r;__tcfapi('addEventListener',2,function(t,ok){r=[\
            ok,t.eventStatus,t.tcString,t.listenerId,t.cmpStatus,\
            t.purpose.consents[1],t.purpose.consents[10],\
            t.vendor.consents[5],t.specialFeatureOptins[1],\
            t.publisherCC,t.isServiceSpecific]})",
    )
    .unwrap();
    assert_eq!(
        ev(&mut e, "r.join(',')"),
        "true,tcloaded,TC_TEST,1,loaded,false,false,false,false,AA,false"
    );
}

/// `AcceptAll` → bits à vrai ; hors `maxVendorId` → absent.
#[test]
fn accept_all_sets_consent_bits() {
    let mut e = engine_policy(CmpPolicy::AcceptAll);
    e.eval(
        "var r;__tcfapi('addEventListener',2,function(t,ok){r=[\
            ok,t.purpose.consents[3],t.vendor.consents[12],\
            typeof t.vendor.consents[13],t.specialFeatureOptins[12],\
            t.gdprApplies]})",
    )
    .unwrap();
    assert_eq!(
        ev(&mut e, "r.join(',')"),
        "true,true,true,undefined,true,true"
    );
}

/// `getTCData`/`getInAppTCData` → `tcData` sans `listenerId`.
#[test]
fn get_tcdata_and_in_app_serve_decision() {
    let mut e = engine_policy(CmpPolicy::RejectAll);
    e.eval("var a;__tcfapi('getTCData',2,function(t,ok){a=[ok,t.eventStatus,t.tcString,t.listenerId]})")
        .unwrap();
    assert_eq!(ev(&mut e, "a.join(',')"), "true,tcloaded,TC_TEST,");
    e.eval("var b;__tcfapi('getInAppTCData',2,function(t,ok){b=[ok,t.cmpStatus]})")
        .unwrap();
    assert_eq!(ev(&mut e, "b.join(',')"), "true,loaded");
}

/// `listenerId` croissant par listener (spec IAB).
#[test]
fn listener_ids_increment() {
    let mut e = engine_policy(CmpPolicy::RejectAll);
    e.eval(
        "var ids=[];__tcfapi('addEventListener',2,function(t){ids.push(t.listenerId)});\
            __tcfapi('addEventListener',2,function(t){ids.push(t.listenerId)})",
    )
    .unwrap();
    assert_eq!(ev(&mut e, "ids.join(',')"), "1,2");
}

/// `getVendorList` → GVL minimale honnête (version 0).
#[test]
fn get_vendor_list_minimal() {
    let mut e = engine_policy(CmpPolicy::RejectAll);
    e.eval("var r;__tcfapi('getVendorList',2,function(g,ok){r=[ok,g.vendorListVersion,g.purposes[1].name.length>0]})")
        .unwrap();
    assert_eq!(ev(&mut e, "r.join(',')"), "true,0,true");
}

/// Commande inconnue → `(null,false)`, moteur vivant.
#[test]
fn unknown_command_degrades() {
    let mut e = engine_policy(CmpPolicy::RejectAll);
    e.eval("var r;__tcfapi('frobnicate',2,function(d,ok){r=[d===null,ok]})")
        .unwrap();
    assert_eq!(ev(&mut e, "r.join(',')"), "true,false");
    // Sans callback → aucun crash.
    e.eval("__tcfapi('ping',2)").unwrap();
}

// ----------------------------------------------------------------
// Résilience aux redéfinitions (props `FIXED`)
// ----------------------------------------------------------------

/// `window.__tcfapi = …` et `Object.defineProperty` ne
/// remplacent pas le résolveur — `FIXED` non-rewritable,
/// non-configurable.
#[test]
fn tcfapi_survives_redefinitions() {
    let mut e = engine_policy(CmpPolicy::RejectAll);
    // Assignation sloppy sur non-writable → ignorée.
    e.eval("window.__tcfapi = 0").unwrap();
    assert_eq!(ev(&mut e, "typeof __tcfapi"), "function");
    // `defineProperty` sur non-configurable → jette ou échoue,
    // la prop reste notre résolveur.
    e.eval("try{Object.defineProperty(window,'__tcfapi',{value:0,configurable:true,writable:true})}catch(_){}")
        .unwrap();
    assert_eq!(ev(&mut e, "typeof __tcfapi"), "function");
    e.eval("var p;__tcfapi('ping',2,function(d,ok){p=[ok,d.cmpStatus]})")
        .unwrap();
    assert_eq!(ev(&mut e, "p.join(',')"), "true,loaded");
}

/// `didomiOnReady = []` (réinit site) n'écrase pas la file
/// instrumentée — `push` appelle toujours immédiatement.
#[test]
fn didomi_queue_survives_reinit() {
    let mut e = engine_policy(CmpPolicy::RejectAll);
    e.eval("var hit=false; window.didomiOnReady=[]; didomiOnReady.push(function(){hit=true})")
        .unwrap();
    assert_eq!(ev(&mut e, "hit"), "true");
}

// ----------------------------------------------------------------
// §B-70 / JS-C260 — accesseurs absorbants
// ----------------------------------------------------------------

/// L'idiome réel `x = x || []` des SDKs (wamiz vol 503 —
/// `TypeError: cannot write '__tcfapi'` tuait le stub IAB
/// sur la prop FIXED) est absorbé silencieusement.
#[test]
fn tcfapi_absorbs_sdk_self_init() {
    let mut e = engine_policy(CmpPolicy::RejectAll);
    e.eval("window.__tcfapi = window.__tcfapi || []").unwrap();
    assert_eq!(ev(&mut e, "typeof __tcfapi"), "function");
    // Hostile absorbé pareil — pas de TypeError, pas d'effet.
    e.eval("window.__tcfapi = 0").unwrap();
    e.eval("var p;__tcfapi('ping',2,function(d,ok){p=[ok,d.cmpStatus]})")
        .unwrap();
    assert_eq!(ev(&mut e, "p.join(',')"), "true,loaded");
}

/// `didomiOnReady = didomiOnReady || []` (madmoizelle vol
/// 503 — `cannot write 'didomiOnReady'` ×2) : absorbé, et le
/// setter **draine** les callbacks d'une file réassignée.
#[test]
fn didomi_onready_absorbs_and_drains() {
    let mut e = engine_policy(CmpPolicy::RejectAll);
    e.eval("window.didomiOnReady = window.didomiOnReady || []")
        .unwrap();
    e.eval("var hit=0; didomiOnReady.push(function(){hit++})")
        .unwrap();
    // File pré-remplie réassignée → callbacks drainés.
    e.eval("didomiOnReady = [function(){hit++},function(){hit++}]")
        .unwrap();
    assert_eq!(ev(&mut e, "hit"), "3");
    // La prop sert toujours notre tableau instrumenté.
    e.eval("didomiOnReady.push(function(){hit++})").unwrap();
    assert_eq!(ev(&mut e, "hit"), "4");
}

/// `Didomi.notice.isVisible` — surface lue par les bundles
/// (wamiz vol 504 : `isVisible is not a function` tuait un
/// script site) ; consentement décidé → `false`.
#[test]
fn didomi_notice_is_visible_bounded() {
    let mut e = engine_policy(CmpPolicy::RejectAll);
    assert_eq!(ev(&mut e, "Didomi.notice.isVisible()"), "false");
    assert_eq!(ev(&mut e, "typeof Didomi.notice.isVisible"), "function");
}

/// `isRegulationApplied`/`getCurrentUserStatus` — méthodes SDK
/// profondes lues par les bundles (vol 584 : bfmtv
/// `isRegulationApplied`, wamiz `getCurrentUserStatus` —
/// `is not a function` sur l'objet `getUserConsentStatusFor*`).
/// Le statut rend la structure spec Didomi : `purposes`/`vendors`
/// sous `consent`/`legitimate_interest` avec `enabled`/`disabled`/
/// `all` en arrays honnêtement vides (aucun id requis connu).
#[test]
fn didomi_deep_methods_bounded() {
    let mut e = engine_policy(CmpPolicy::RejectAll);
    assert_eq!(ev(&mut e, "typeof Didomi.isRegulationApplied"), "function");
    assert_eq!(ev(&mut e, "Didomi.isRegulationApplied()"), "true");
    assert_eq!(ev(&mut e, "typeof Didomi.getCurrentUserStatus"), "function");
    assert_eq!(
        ev(
            &mut e,
            "var s=Didomi.getCurrentUserStatus();\
             [Array.isArray(s.purposes.consent.enabled),\
              Array.isArray(s.vendors.legitimate_interest.disabled),\
              Array.isArray(s.purposes.consent.all),\
              s.purposes.consent.enabled.length,\
              typeof s.user_id, s.version].join('|')"
        ),
        "true|true|true|0|string|2"
    );
}

/// `OptanonWrapper` — le setter appelle le callback ET le
/// mémorise : le getter le rend ensuite au SDK.
#[test]
fn optanon_wrapper_get_returns_assigned() {
    let mut e = engine_policy(CmpPolicy::RejectAll);
    e.eval("var hit=false; window.OptanonWrapper=function(){hit=true}")
        .unwrap();
    assert_eq!(ev(&mut e, "hit"), "true");
    assert_eq!(ev(&mut e, "typeof OptanonWrapper"), "function");
}

// ----------------------------------------------------------------
// APIs sœurs bornées
// ----------------------------------------------------------------

/// `__sdcmpapi` (Sirdata) — mêmes commandes IAB que
/// `__tcfapi`.
#[test]
fn sdcmpapi_serves_same_decision() {
    let mut e = engine_policy(CmpPolicy::RejectAll);
    e.eval("var p;__sdcmpapi('ping',2,function(d,ok){p=[ok,d.cmpLoaded,d.cmpStatus]})")
        .unwrap();
    assert_eq!(ev(&mut e, "p.join(',')"), "true,true,loaded");
    e.eval("var r;__sdcmpapi('addEventListener',2,function(t,ok){r=[ok,t.eventStatus,t.purpose.consents[2]]})")
        .unwrap();
    assert_eq!(ev(&mut e, "r.join(',')"), "true,tcloaded,false");
}

/// `__cmp` (TCF v1) — `ping` + `getVendorConsents` bornés.
#[test]
fn cmp_v1_basics() {
    let mut e = engine_policy(CmpPolicy::RejectAll);
    e.eval("var p;__cmp('ping',null,function(d,ok){p=[ok,d.cmpLoaded]})")
        .unwrap();
    assert_eq!(ev(&mut e, "p.join(',')"), "true,true");
    e.eval("var r;__cmp('getVendorConsents',null,function(d,ok){r=[ok,d.purposeConsents[1],d.vendorConsents[5]]})")
        .unwrap();
    assert_eq!(ev(&mut e, "r.join(',')"), "true,false,false");
}

/// `__uspapi` — `uspString` de la politique (`1YYY` refus,
/// `1YNN` accept).
#[test]
fn uspapi_reflects_policy() {
    let mut e = engine_policy(CmpPolicy::RejectAll);
    e.eval("var r;__uspapi('getUSPData',1,function(d,ok){r=[ok,d.uspString]})")
        .unwrap();
    assert_eq!(ev(&mut e, "r.join(',')"), "true,1YYY");
    let mut e = engine_policy(CmpPolicy::AcceptAll);
    e.eval("var r;__uspapi('getUSPData',1,function(d,ok){r=[d.uspString]})")
        .unwrap();
    assert_eq!(ev(&mut e, "r.join(',')"), "1YNN");
}

/// `__gpp` — `ping` → descripteur chargé CMP-less
/// (`supportedAPIs` vide — déclarer des sections inservables
/// provoquait `GPP section uspv1 is not supported` côté site,
/// §B-14 webtoons vol 529) ; commandes spec répondues
/// honnêtement `(donnée vide, true)`.
#[test]
fn gpp_ping_loaded() {
    let mut e = engine_policy(CmpPolicy::RejectAll);
    e.eval("var r;__gpp('ping',function(d,ok){r=[ok,d.cmpStatus,d.gppVersion,d.signalStatus,d.supportedAPIs.length]})")
        .unwrap();
    assert_eq!(ev(&mut e, "r.join(',')"), "true,loaded,1.1,ready,0");
    e.eval("var s;__gpp('getSection',function(d,ok){s=[d.length,ok]})")
        .unwrap();
    assert_eq!(ev(&mut e, "s.join(',')"), "0,true");
    e.eval("var h;__gpp('hasSection',function(d,ok){h=[d,ok]})")
        .unwrap();
    assert_eq!(ev(&mut e, "h.join(',')"), "false,true");
    e.eval("var f;__gpp('getField',function(d,ok){f=[d===null,ok]})")
        .unwrap();
    assert_eq!(ev(&mut e, "f.join(',')"), "true,true");
    e.eval("var u;__gpp('unknown',function(d,ok){u=[d===null,ok]})")
        .unwrap();
    assert_eq!(ev(&mut e, "u.join(',')"), "true,false");
}

/// `OneTrust` — `OnetrustActiveGroups` de la politique ;
/// `OptanonWrapper = fn` appelle le callback d'emblée.
#[test]
fn onetrust_surface() {
    let mut e = engine_policy(CmpPolicy::RejectAll);
    assert_eq!(ev(&mut e, "OnetrustActiveGroups"), ",C0001,");
    e.eval("var hit=false; window.OptanonWrapper=function(){hit=true}")
        .unwrap();
    assert_eq!(ev(&mut e, "hit"), "true");

    let mut e = engine_policy(CmpPolicy::AcceptAll);
    assert_eq!(
        ev(&mut e, "OnetrustActiveGroups"),
        ",C0001,C0002,C0003,C0004,C0005,"
    );
}

/// Didomi — statuts de la politique, `isConsentRequired`
/// faux (consentement exprimé), `didomiState` présent.
#[test]
fn didomi_surface() {
    let mut e = engine_policy(CmpPolicy::RejectAll);
    assert_eq!(
        ev(&mut e, "Didomi.getUserConsentStatusForPurpose('cookies')"),
        "false"
    );
    assert_eq!(ev(&mut e, "Didomi.isConsentRequired()"), "false");
    assert_eq!(ev(&mut e, "typeof didomiState"), "object");
    e.eval("var hit=false; didomiOnReady.push(function(){hit=true})")
        .unwrap();
    assert_eq!(ev(&mut e, "hit"), "true");

    let mut e = engine_policy(CmpPolicy::AcceptAll);
    assert_eq!(
        ev(&mut e, "Didomi.getUserConsentStatusForVendor(123)"),
        "true"
    );
}

/// `_sp_`/`Osano` — présence bornée lue par les gates.
#[test]
fn sourcepoint_and_osano_present() {
    let mut e = engine_policy(CmpPolicy::RejectAll);
    assert_eq!(
        ev(&mut e, "typeof _sp_.loadPrivacyManagerModal"),
        "function"
    );
    assert_eq!(ev(&mut e, "Osano.cm.markedForConsent('x')"), "true");
    assert_eq!(ev(&mut e, "typeof __tcfapiLocator"), "object");
}

// ----------------------------------------------------------------
// Contextes enfants — la décision est engine-globale
// ----------------------------------------------------------------

/// Un realm enfant (`context_new` + `install_dom`) pose le
/// même résolveur — `install_misc` lit `e.cmp` partagé.
#[test]
fn child_context_inherits_decision() {
    let mut e = engine_policy(CmpPolicy::RejectAll);
    let child = e.context_new("https://child.example/f", None);
    let cd = Rc::new(RefCell::new(Dom::from_html(PAGE)));
    e.install_dom(&cd);
    e.eval("var p;__tcfapi('ping',2,function(d,ok){p=[ok,d.cmpStatus,d.cmpLoaded]})")
        .unwrap();
    assert_eq!(ev(&mut e, "p.join(',')"), "true,loaded,true");
    // Le parent garde son résolveur — état partagé, realms isolés.
    e.context_switch(0);
    e.eval("var q;__tcfapi('ping',2,function(d,ok){q=[ok,d.cmpStatus]})")
        .unwrap();
    assert_eq!(ev(&mut e, "q.join(',')"), "true,loaded");
    let _ = child;
}
