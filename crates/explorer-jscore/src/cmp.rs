// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Consentement CMP côté JS (JS-C259, §B-4 bras CMP —
//! ADR-0031).
//!
//! Le stub `__tcfapi` honnête (JS-C213) reste le comportement
//! par défaut — [`CmpPolicy::None`]. Sous politique résolue
//! (`RejectAll`/`AcceptAll`, poussée par
//! `SessionConfig::consent_policy` → [`Engine::set_cmp_consent`]
//! avant `install_dom`), `__tcfapi` devient un **résolveur** :
//! descripteur CMP chargée, `addEventListener` invoque le
//! callback avec un `tcData` formé IAB v2
//! (`eventStatus:'tcloaded'`, `listenerId` croissant), les
//! commandes data répondent la décision de la politique.
//!
//! # Résilience (bornage « redéfinition »)
//!
//! Les props sont posées `FIXED` (non-rewritable,
//! non-configurable) : un SDK CMP qui réassigne
//! `window.__tcfapi` (ou un site qui pose
//! `window.didomiOnReady = []`) n'écrase pas le résolveur —
//! la décision de politique reste la seule source servie.
//!
//! # APIs sœurs (bornées aux call-shapes observés au vol)
//!
//! `__cmp` (TCF v1), `__uspapi`, `__gpp` (`ping`),
//! `__sdcmpapi` (Sirdata/Sourcepoint Dialogue — mêmes
//! commandes IAB), `OnetrustActiveGroups`/`OptanonWrapper`
//! (`OneTrust` — l'assignation appelle le callback,
//! consentement déjà décidé), `Didomi`/`didomiState`/
//! `didomiOnReady` (`push` intercepté → appel immédiat),
//! `_sp_`, `Osano`, `__tcfapiLocator` (marqueur — le pont
//! `postMessage` `__tcfapiCall` reste un trou documenté).

use crate::heap::JsResult;
use crate::value::{ObjId, Value};
use crate::Engine;

/// Décision de consentement CMP portée par le moteur —
/// injectée par la session ([`Engine::set_cmp_consent`],
/// miroir de `explorer_net::consent::ConsentPolicy`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CmpPolicy {
    /// Aucune CMP déclarée — `__tcfapi` honnête CMP-less
    /// (statu quo JS-C213).
    #[default]
    None,
    /// « Tout refuser » — purposes/vendors bitfields à faux.
    RejectAll,
    /// « Tout accepter » — outil de diagnostic.
    AcceptAll,
}

/// État consenti publié au moteur — `tc_string` doit être la
/// valeur `euconsent-v2` réellement dans le jar (la session
/// la relit — cohérence `document.cookie` ↔ `tcData`).
#[derive(Debug, Clone, Default)]
pub struct CmpConsent {
    /// Décision (`None` = comportement honnête historique).
    pub policy: CmpPolicy,
    /// TC string IAB v2 (segment core) — `tcString` des
    /// `tcData`/`getVendorList`.
    pub tc_string: String,
    /// `maxVendorId` des bitfields vendor servis.
    pub max_vendor_id: u16,
    /// `gdprApplies` des réponses (TCF applicable à
    /// l'utilisateur — les sites EU du corpus s'attendent
    /// à `true` quand la politique est résolue).
    pub gdpr_applies: bool,
}

impl Engine {
    /// Injecte la décision CMP de la session — **avant**
    /// `install_dom` : les props consentement sont posées au
    /// chargement du miroir (`install_misc` → `install_cmp`).
    pub fn set_cmp_consent(&mut self, c: CmpConsent) {
        self.cmp = c;
    }
}

/// Pose `__tcfapi` (+ les APIs sœurs sous politique résolue)
/// — appelé par `install_misc` sur chaque global (top et
/// contextes enfants : la décision est engine-globale).
pub(crate) fn install_cmp(e: &mut Engine) {
    let g = e.global;
    let tcf = e.native_fn("__tcfapi", tcfapi);
    if matches!(e.cmp.policy, CmpPolicy::None) {
        // Honnête (statu quo JS-C213) — prop ordinaire, les
        // SDKs réels peuvent la remplacer.
        e.set_own(g, "__tcfapi", tcf, false);
        return;
    }
    // `FIXED` : ni assignation ni `Object.defineProperty` ne
    // remplacent le résolveur (résilience aux redéfinitions —
    // le SDK CMP garde la main sur rien).
    e.set_own_attrs(g, "__tcfapi", tcf, crate::heap::PropAttrs::FIXED);
    install_cmp_siblings(e, g);
}

/// APIs sœurs des CMPs rencontrées au vol — chacune bornée
/// aux entrées réellement lues (pas la spec entière).
fn install_cmp_siblings(e: &mut Engine, g: ObjId) {
    let fixed = crate::heap::PropAttrs::FIXED;
    // `__sdcmpapi` (Sirdata / Sourcepoint Dialogue) — même
    // signature triple que `__tcfapi`, mêmes commandes.
    let sdc = e.native_fn("__sdcmpapi", tcfapi);
    e.set_own_attrs(g, "__sdcmpapi", sdc, fixed);
    // `__cmp` (TCF v1) / `__uspapi` (USP) / `__gpp` (GPP).
    let v1 = e.native_fn("__cmp", cmp_v1);
    e.set_own_attrs(g, "__cmp", v1, fixed);
    let usp = e.native_fn("__uspapi", uspapi);
    e.set_own_attrs(g, "__uspapi", usp, fixed);
    let gpp = e.native_fn("__gpp", gpp);
    e.set_own_attrs(g, "__gpp", gpp, fixed);
    // `__tcfapiLocator` — marqueur IAB (le pont postMessage
    // `__tcfapiCall` n'est pas implémenté — trou borné).
    let loc = e.new_object();
    e.set_own_attrs(g, "__tcfapiLocator", Value::Obj(loc), fixed);
    // OneTrust — `OnetrustActiveGroups` lu tel quel ; le
    // « nécessaire » `C0001` reste actif même au refus (OT
    // réel idem), les autres groupes suivent la politique.
    let agreed = matches!(e.cmp.policy, CmpPolicy::AcceptAll);
    let groups = if agreed {
        ",C0001,C0002,C0003,C0004,C0005,"
    } else {
        ",C0001,"
    };
    let gv = e.str_val(groups);
    e.set_own_attrs(g, "OnetrustActiveGroups", gv, fixed);
    // `OptanonWrapper` — le site **assigne** le callback ;
    // consentement déjà décidé → OT l'appellerait d'emblée,
    // notre setter l'invoque immédiatement (borné : non
    // callable ignoré, erreur du cb dégradée).
    let setter = e.native_fn("OptanonWrapper", optanon_set);
    if let Some(o) = e.heap.obj_mut(g) {
        o.define_accessor(
            "OptanonWrapper",
            Value::Undefined,
            setter,
            crate::heap::PropAttrs {
                writable: false,
                enumerable: true,
                configurable: true,
            },
        );
    }
    // `Didomi` — surface lue par les gates observés :
    // statuts de consentement, `isConsentRequired`, `notice`.
    let d = e.new_object();
    e.add_method(d, "getUserConsentStatusForPurpose", didomi_status);
    e.add_method(d, "getUserConsentStatusForVendor", didomi_status);
    e.add_method(d, "getUserConsentStatus", didomi_status);
    e.add_method(d, "getUserConsentStatusForVendorOrAll", didomi_status);
    e.add_method(d, "isConsentRequired", didomi_not_required);
    e.add_method(d, "isUserConsentStatusPartial", didomi_not_required);
    e.add_method(d, "setUserConsentStatus", cmp_noop);
    e.add_method(d, "getRequiredPurposeIds", empty_array);
    e.add_method(d, "getRequiredVendorIds", empty_array);
    let notice = e.new_object();
    e.add_method(notice, "show", cmp_noop);
    e.add_method(notice, "hide", cmp_noop);
    e.add_method(notice, "toggle", cmp_noop);
    e.set_own(d, "notice", Value::Obj(notice), true);
    e.set_own_attrs(g, "Didomi", Value::Obj(d), fixed);
    // `didomiState` — présence lue par certains gates.
    let ds = e.new_object();
    e.set_own(ds, "didomiExperimentId", Value::Null, true);
    e.set_own(ds, "didomiExperimentUserGroup", Value::Null, true);
    e.set_own(ds, "didomiTimestamp", Value::Num(0.0), true);
    e.set_own_attrs(g, "didomiState", Value::Obj(ds), fixed);
    // `didomiOnReady` — le site `push` ses callbacks ; le SDK
    // réel les draine. Notre `push` propre appelle chaque
    // fonction immédiatement (consentement déjà décidé) —
    // la prop `FIXED` protège le `didomiOnReady = []` des
    // sites qui réinitialisent la file.
    let arr = e.new_array(Vec::new());
    if let Value::Obj(aid) = arr {
        let push = e.native_fn("didomiOnReady.push", didomi_push);
        e.set_own_attrs(aid, "push", push, fixed);
    }
    e.set_own_attrs(g, "didomiOnReady", arr, fixed);
    // `_sp_` (Sourcepoint) — objet de présence + hook de
    // privacy manager (no-op borné).
    let sp = e.new_object();
    e.add_method(sp, "loadPrivacyManagerModal", cmp_noop);
    e.add_method(sp, "loadPrivacyManagerOOB", cmp_noop);
    let spc = e.new_object();
    e.set_own(spc, "accountId", Value::Num(0.0), true);
    e.set_own(sp, "config", Value::Obj(spc), true);
    e.set_own_attrs(g, "_sp_", Value::Obj(sp), fixed);
    // `Osano` — `cm` lu pour ses flags d'état.
    let os = e.new_object();
    let cm = e.new_object();
    e.add_method(cm, "show", cmp_noop);
    e.add_method(cm, "hide", cmp_noop);
    e.add_method(cm, "markedForConsent", osano_marked);
    e.add_method(cm, "getConsents", empty_obj);
    e.set_own(os, "cm", Value::Obj(cm), true);
    e.set_own_attrs(g, "Osano", Value::Obj(os), fixed);
}

// ----------------------------------------------------------------
// `__tcfapi` — résolveur IAB TCF v2 (ou stub honnête `None`)
// ----------------------------------------------------------------

/// `window.__tcfapi(command, version, callback, parameter)`.
fn tcfapi(e: &mut Engine, t: Value, a: &[Value]) -> JsResult<Value> {
    if matches!(e.cmp.policy, CmpPolicy::None) {
        return tcfapi_honest(e, t, a);
    }
    let cmd = match a.first() {
        Some(v) => e.to_js_string(*v)?,
        None => String::new(),
    };
    let cb = arg(a, 2);
    if !e.is_callable(cb) {
        return Ok(Value::Undefined);
    }
    match cmd.as_str() {
        "ping" => {
            let o = ping_descriptor(e);
            e.call_value(cb, Value::Undefined, &[Value::Obj(o), Value::Bool(true)])?;
        }
        "addEventListener" => {
            e.misc.cmp_seq += 1;
            let id = e.misc.cmp_seq;
            let data = tc_data(e, Some(id));
            e.call_value(cb, Value::Undefined, &[Value::Obj(data), Value::Bool(true)])?;
        }
        "removeEventListener" => {
            e.call_value(cb, Value::Undefined, &[Value::Bool(true)])?;
        }
        "getTCData" | "getInAppTCData" => {
            let data = tc_data(e, None);
            e.call_value(cb, Value::Undefined, &[Value::Obj(data), Value::Bool(true)])?;
        }
        "getVendorList" => {
            let gvl = gvl_stub(e);
            e.call_value(cb, Value::Undefined, &[Value::Obj(gvl), Value::Bool(true)])?;
        }
        "getCustomVendorConsents" => {
            let o = e.new_object();
            let c = e.new_object();
            e.set_own(o, "consents", Value::Obj(c), true);
            let gr = e.new_object();
            e.set_own(o, "grants", Value::Obj(gr), true);
            e.call_value(cb, Value::Undefined, &[Value::Obj(o), Value::Bool(true)])?;
        }
        _ => {
            e.call_value(cb, Value::Undefined, &[Value::Null, Value::Bool(false)])?;
        }
    }
    Ok(Value::Undefined)
}

/// Stub honnête CMP-less (JS-C213) — comportement
/// `CmpPolicy::None`, inchangé.
fn tcfapi_honest(e: &mut Engine, _t: Value, a: &[Value]) -> JsResult<Value> {
    let cmd = match a.first() {
        Some(v) => e.to_js_string(*v)?,
        None => String::new(),
    };
    let cb = arg(a, 2);
    if !e.is_callable(cb) {
        return Ok(Value::Undefined);
    }
    match cmd.as_str() {
        "ping" => {
            let o = e.new_object();
            e.set_own(o, "gdprApplies", Value::Bool(false), true);
            e.set_own(o, "cmpLoaded", Value::Bool(false), true);
            for (k, s) in [
                ("cmpStatus", "stub"),
                ("apiVersion", "2.2"),
                ("displayStatus", "disabled"),
            ] {
                let sv = e.str_val(s);
                e.set_own(o, k, sv, true);
            }
            for (k, n) in [
                ("cmpId", 0.0),
                ("cmpVersion", 0.0),
                ("tcfPolicyVersion", 4.0),
                ("gvlVersion", 0.0),
            ] {
                e.set_own(o, k, Value::Num(n), true);
            }
            e.call_value(cb, Value::Undefined, &[Value::Obj(o), Value::Bool(true)])?;
        }
        "removeEventListener" => {
            e.call_value(cb, Value::Undefined, &[Value::Bool(true)])?;
        }
        _ => {
            e.call_value(cb, Value::Undefined, &[Value::Null, Value::Bool(false)])?;
        }
    }
    Ok(Value::Undefined)
}

/// Descripteur `ping` d'une CMP chargée — `cmpLoaded:true`,
/// `cmpStatus:'loaded'` (les autres champs restent honnêtes :
/// `cmpId 0`, pas de GVL réelle).
fn ping_descriptor(e: &mut Engine) -> ObjId {
    let o = e.new_object();
    e.set_own(o, "gdprApplies", Value::Bool(e.cmp.gdpr_applies), true);
    e.set_own(o, "cmpLoaded", Value::Bool(true), true);
    for (k, s) in [
        ("cmpStatus", "loaded"),
        ("apiVersion", "2.2"),
        ("displayStatus", "disabled"),
    ] {
        let sv = e.str_val(s);
        e.set_own(o, k, sv, true);
    }
    for (k, n) in [
        ("cmpId", 0.0),
        ("cmpVersion", 1.0),
        ("tcfPolicyVersion", 4.0),
        ("gvlVersion", 0.0),
    ] {
        e.set_own(o, k, Value::Num(n), true);
    }
    o
}

/// `tcData` IAB v2 formé sur la décision de politique —
/// `eventStatus:'tcloaded'` (consentement déjà décidé),
/// bitfields à `agreed` pour tous les ids servis.
fn tc_data(e: &mut Engine, listener: Option<u32>) -> ObjId {
    let agreed = matches!(e.cmp.policy, CmpPolicy::AcceptAll);
    let o = e.new_object();
    let tcs = e.str_val(e.cmp.tc_string.clone());
    e.set_own(o, "tcString", tcs, true);
    let es = e.str_val("tcloaded");
    e.set_own(o, "eventStatus", es, true);
    let cs = e.str_val("loaded");
    e.set_own(o, "cmpStatus", cs, true);
    let cl = e.str_val("EN");
    e.set_own(o, "consentLanguage", cl, true);
    let pcc = e.str_val("AA");
    e.set_own(o, "publisherCC", pcc, true);
    for (k, n) in [
        ("cmpId", 0.0),
        ("cmpVersion", 1.0),
        ("tcfPolicyVersion", 4.0),
    ] {
        e.set_own(o, k, Value::Num(n), true);
    }
    if let Some(id) = listener {
        e.set_own(o, "listenerId", Value::Num(f64::from(id)), true);
    }
    e.set_own(o, "gdprApplies", Value::Bool(e.cmp.gdpr_applies), true);
    e.set_own(o, "isServiceSpecific", Value::Bool(false), true);
    e.set_own(o, "useNonStandardStacks", Value::Bool(false), true);
    e.set_own(o, "purposeOneTreatment", Value::Bool(false), true);
    // `specialFeatureOptins` — ids 1..=12.
    let sfo = bool_map(e, 1, 12, agreed);
    e.set_own(o, "specialFeatureOptins", Value::Obj(sfo), true);
    // `purpose` — consents + legitimateInterests, ids 1..=10.
    let purpose = e.new_object();
    let pc = bool_map(e, 1, 10, agreed);
    e.set_own(purpose, "consents", Value::Obj(pc), true);
    let pli = bool_map(e, 1, 10, agreed);
    e.set_own(purpose, "legitimateInterests", Value::Obj(pli), true);
    e.set_own(o, "purpose", Value::Obj(purpose), true);
    // `vendor` — ids 1..=maxVendorId.
    let vendor = e.new_object();
    let vc = bool_map(e, 1, u32::from(e.cmp.max_vendor_id), agreed);
    e.set_own(vendor, "consents", Value::Obj(vc), true);
    let vli = bool_map(e, 1, u32::from(e.cmp.max_vendor_id), agreed);
    e.set_own(vendor, "legitimateInterests", Value::Obj(vli), true);
    e.set_own(o, "vendor", Value::Obj(vendor), true);
    // `publisher` — maps vides (pas de restrictions posées).
    let pub_ = e.new_object();
    for k in ["consents", "legitimateInterests"] {
        let m = e.new_object();
        e.set_own(pub_, k, Value::Obj(m), true);
    }
    let cp = e.new_object();
    for k in ["consents", "legitimateInterests"] {
        let m = e.new_object();
        e.set_own(cp, k, Value::Obj(m), true);
    }
    e.set_own(pub_, "customPurpose", Value::Obj(cp), true);
    let rst = e.new_object();
    e.set_own(pub_, "restrictions", Value::Obj(rst), true);
    e.set_own(o, "publisher", Value::Obj(pub_), true);
    // `outOfBand` — maps vides.
    let oob = e.new_object();
    for k in ["allowedVendors", "disclosedVendors"] {
        let m = e.new_object();
        e.set_own(oob, k, Value::Obj(m), true);
    }
    e.set_own(o, "outOfBand", Value::Obj(oob), true);
    o
}

/// Objet `{i: v}` pour `i ∈ [from..=to]` — bitfield IAB.
fn bool_map(e: &mut Engine, from: u32, to: u32, v: bool) -> ObjId {
    let o = e.new_object();
    for i in from..=to {
        e.set_own(o, &i.to_string(), Value::Bool(v), true);
    }
    o
}

/// Noms IAB publics des purposes v2 — pour les réponses
/// `getVendorList` (spec publique, pas de texte propriétaire).
const PURPOSE_NAMES: [&str; 10] = [
    "Store and/or access information on a device",
    "Select basic ads",
    "Create a personalised ads profile",
    "Select personalised ads",
    "Create a personalised content profile",
    "Select personalised content",
    "Measure ad performance",
    "Measure content performance",
    "Apply market research to generate audience insights",
    "Develop and improve products",
];

/// `getVendorList` — GVL minimale honnête : `vendorListVersion:0`
/// (aucune vraie liste derrière), noms de purposes de la spec
/// IAB publique pour les re-décodeurs qui les affichent.
fn gvl_stub(e: &mut Engine) -> ObjId {
    let o = e.new_object();
    for (k, n) in [
        ("gvlSpecificationVersion", 2.0),
        ("vendorListVersion", 0.0),
        ("tcfPolicyVersion", 4.0),
    ] {
        e.set_own(o, k, Value::Num(n), true);
    }
    let lu = e.str_val("2026-01-01T00:00:00Z");
    e.set_own(o, "lastUpdated", lu, true);
    let purposes = e.new_object();
    for (i, name) in PURPOSE_NAMES.iter().enumerate() {
        let p = e.new_object();
        e.set_own(p, "id", Value::Num(i as f64 + 1.0), true);
        let nv = e.str_val(*name);
        e.set_own(p, "name", nv, true);
        let ill = e.new_array(Vec::new());
        e.set_own(p, "illustrations", ill, true);
        e.set_own(purposes, &(i + 1).to_string(), Value::Obj(p), true);
    }
    e.set_own(o, "purposes", Value::Obj(purposes), true);
    for k in [
        "specialPurposes",
        "features",
        "specialFeatures",
        "dataCategories",
    ] {
        let m = e.new_object();
        e.set_own(o, k, Value::Obj(m), true);
    }
    let st = e.new_object();
    e.set_own(o, "stacks", Value::Obj(st), true);
    let vendors = e.new_object();
    e.set_own(o, "vendors", Value::Obj(vendors), true);
    o
}

// ----------------------------------------------------------------
// APIs sœurs — natives bornées
// ----------------------------------------------------------------

/// `__cmp(command, parameter, callback)` — TCF v1 minimal :
/// `ping`/`getConsentData`/`getVendorConsents`/
/// `getPublisherConsents` répondent la décision.
fn cmp_v1(e: &mut Engine, _t: Value, a: &[Value]) -> JsResult<Value> {
    let cmd = match a.first() {
        Some(v) => e.to_js_string(*v)?,
        None => String::new(),
    };
    let cb = arg(a, 2);
    if !e.is_callable(cb) {
        return Ok(Value::Undefined);
    }
    let agreed = matches!(e.cmp.policy, CmpPolicy::AcceptAll);
    let gdpr = e.cmp.gdpr_applies;
    match cmd.as_str() {
        "ping" => {
            let o = e.new_object();
            e.set_own(o, "gdprAppliesGlobally", Value::Bool(false), true);
            e.set_own(o, "cmpLoaded", Value::Bool(true), true);
            e.call_value(cb, Value::Undefined, &[Value::Obj(o), Value::Bool(true)])?;
        }
        "getConsentData" => {
            let o = e.new_object();
            e.set_own(o, "gdprApplies", Value::Bool(gdpr), true);
            e.set_own(o, "hasGlobalScope", Value::Bool(false), true);
            let cd = e.str_val("");
            e.set_own(o, "consentData", cd, true);
            e.call_value(cb, Value::Undefined, &[Value::Obj(o), Value::Bool(true)])?;
        }
        "getVendorConsents" | "getPublisherConsents" => {
            let o = e.new_object();
            e.set_own(o, "gdprApplies", Value::Bool(gdpr), true);
            e.set_own(o, "hasGlobalScope", Value::Bool(false), true);
            let pc = bool_map(e, 1, 5, agreed);
            e.set_own(o, "purposeConsents", Value::Obj(pc), true);
            let vc = bool_map(e, 1, u32::from(e.cmp.max_vendor_id), agreed);
            e.set_own(o, "vendorConsents", Value::Obj(vc), true);
            e.call_value(cb, Value::Undefined, &[Value::Obj(o), Value::Bool(true)])?;
        }
        _ => {
            e.call_value(cb, Value::Undefined, &[Value::Null, Value::Bool(false)])?;
        }
    }
    Ok(Value::Undefined)
}

/// `__uspapi(command, version, callback)` — USP/CCPA minimal :
/// `getUSPData` rend la `uspString` de la politique.
fn uspapi(e: &mut Engine, _t: Value, a: &[Value]) -> JsResult<Value> {
    let cmd = match a.first() {
        Some(v) => e.to_js_string(*v)?,
        None => String::new(),
    };
    let cb = arg(a, 2);
    if !e.is_callable(cb) {
        return Ok(Value::Undefined);
    }
    if cmd == "getUSPData" {
        let o = e.new_object();
        e.set_own(o, "version", Value::Num(1.0), true);
        let us = e.str_val(if matches!(e.cmp.policy, CmpPolicy::AcceptAll) {
            "1YNN"
        } else {
            "1YYY"
        });
        e.set_own(o, "uspString", us, true);
        e.call_value(cb, Value::Undefined, &[Value::Obj(o), Value::Bool(true)])?;
    } else {
        e.call_value(cb, Value::Undefined, &[Value::Null, Value::Bool(false)])?;
    }
    Ok(Value::Undefined)
}

/// `__gpp(command, callback, parameter)` — GPP minimal :
/// `ping` rend le descripteur, `getSection`/`getField` →
/// `null` honnête (sections encodées non implémentées).
fn gpp(e: &mut Engine, _t: Value, a: &[Value]) -> JsResult<Value> {
    let cmd = match a.first() {
        Some(v) => e.to_js_string(*v)?,
        None => String::new(),
    };
    let cb = arg(a, 1);
    if !e.is_callable(cb) {
        return Ok(Value::Undefined);
    }
    if cmd == "ping" {
        let o = e.new_object();
        let gv = e.str_val("1.1");
        e.set_own(o, "gppVersion", gv, true);
        for (k, s) in [
            ("cmpStatus", "loaded"),
            ("cmpDisplayStatus", "hidden"),
            ("signalStatus", "ready"),
        ] {
            let sv = e.str_val(s);
            e.set_own(o, k, sv, true);
        }
        let apis: Vec<Value> = ["5:usp1", "7:usnat", "8:usca"]
            .iter()
            .map(|s| e.str_val(*s))
            .collect();
        let apis = e.new_array(apis);
        e.set_own(o, "supportedAPIs", apis, true);
        let gs = e.str_val("");
        e.set_own(o, "gppString", gs, true);
        let empty = e.new_array(Vec::new());
        e.set_own(o, "sectionList", empty, true);
        let neg = e.new_array(vec![Value::Num(-1.0)]);
        e.set_own(o, "applicableSections", neg, true);
        e.call_value(cb, Value::Undefined, &[Value::Obj(o), Value::Bool(true)])?;
    } else {
        e.call_value(cb, Value::Undefined, &[Value::Null, Value::Bool(false)])?;
    }
    Ok(Value::Undefined)
}

/// Setter `OptanonWrapper` — le site assigne le callback,
/// `OneTrust` le tirerait dès le chargement (consentement déjà
/// décidé) → appel immédiat ; erreur dégradée.
fn optanon_set(e: &mut Engine, _t: Value, a: &[Value]) -> JsResult<Value> {
    if let Some(f) = a.first().copied() {
        if e.is_callable(f) {
            let _ = e.call_value(f, Value::Undefined, &[]);
        }
    }
    Ok(Value::Undefined)
}

/// Statut Didomi = la décision de politique (purpose/vendor).
fn didomi_status(e: &mut Engine, _t: Value, _a: &[Value]) -> JsResult<Value> {
    Ok(Value::Bool(matches!(e.cmp.policy, CmpPolicy::AcceptAll)))
}

/// `isConsentRequired`/`isUserConsentStatusPartial` → faux
/// (consentement déjà décidé — Didomi réel répond `false`
/// quand `notice_behavior` est `expressed`).
fn didomi_not_required(_e: &mut Engine, _t: Value, _a: &[Value]) -> JsResult<Value> {
    Ok(Value::Bool(false))
}

/// `didomiOnReady.push(fn…)` — appelle chaque fonction
/// immédiatement (le SDK réel draine la file au init ;
/// consentement déjà décidé → tir immédiat). Retourne le
/// nombre d'arguments comme `push` retourne `length`.
fn didomi_push(e: &mut Engine, _t: Value, a: &[Value]) -> JsResult<Value> {
    let n = a.len() as f64;
    for f in a {
        if e.is_callable(*f) {
            let _ = e.call_value(*f, Value::Undefined, &[]);
        }
    }
    Ok(Value::Num(n))
}

/// `Osano.cm.markedForConsent` → vrai (le bandeau est marqué
/// « consentement exprimé » — cohérent avec les graines).
fn osano_marked(_e: &mut Engine, _t: Value, _a: &[Value]) -> JsResult<Value> {
    Ok(Value::Bool(true))
}

/// Méthode no-op bornée (`notice.show()`, `_sp_.*`, …).
fn cmp_noop(_e: &mut Engine, _t: Value, _a: &[Value]) -> JsResult<Value> {
    Ok(Value::Undefined)
}

/// `getRequiredPurposeIds`/vendors Didomi → `[]` borné.
fn empty_array(e: &mut Engine, _t: Value, _a: &[Value]) -> JsResult<Value> {
    Ok(e.new_array(Vec::new()))
}

/// `getConsents` Osano → `{}` borné.
fn empty_obj(e: &mut Engine, _t: Value, _a: &[Value]) -> JsResult<Value> {
    Ok(Value::Obj(e.new_object()))
}

/// `args[i]` ou `Undefined` (convention `xhr::arg`).
fn arg(args: &[Value], i: usize) -> Value {
    args.get(i).copied().unwrap_or(Value::Undefined)
}
