// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Verrous de conformité : les vecteurs `schema/golden/` doivent se
//! désérialiser dans les types `proto` — le schéma JSON et le code
//! ne dérivent jamais l'un de l'autre.

use explorer_agent::proto::{self, ErrorCode, Incoming, Method};
use serde_json::{json, Value};
use std::path::Path;

fn golden_dir() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("schema/golden")
}

fn wire_messages() -> Vec<(String, Value)> {
    let mut out = Vec::new();
    let mut entries: Vec<_> = std::fs::read_dir(golden_dir())
        .expect("golden dir")
        .map(|e| e.expect("entry").path())
        .collect();
    entries.sort();
    for path in entries {
        let doc: Value =
            serde_json::from_str(&std::fs::read_to_string(&path).expect("golden lisible"))
                .expect("golden = JSON valide");
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        for step in doc["exchange"].as_array().expect("exchange[]") {
            if let Some(m) = step.get("message") {
                out.push((name.clone(), m.clone()));
            }
        }
    }
    out
}

/// Toute requête fil des golden se désérialise dans `Incoming`.
#[test]
fn golden_requests_parse() {
    let mut n = 0;
    for (file, msg) in wire_messages() {
        if msg.get("id").is_some() && msg.get("method").is_some() {
            let _: Incoming =
                serde_json::from_value(msg).unwrap_or_else(|e| panic!("{file} requête : {e}"));
            n += 1;
        }
    }
    assert!(n >= 10, "couverture golden trop faible : {n} requêtes");
}

/// Chaque `method` golden correspond à une variante connue.
#[test]
fn golden_methods_known() {
    let known = [
        "session.open",
        "session.close",
        "session.ping",
        "tab.open",
        "tab.close",
        "tab.list",
        "nav.goto",
        "nav.reload",
        "nav.back",
        "nav.forward",
        "nav.viewport",
        "snapshot.get",
        "query.select",
        "frame.get",
        "console.get",
        "dom.get",
        "engine.layout",
        "engine.pipeline",
        "engine.anomalies",
        "input.click",
        "input.type",
        "input.hover",
        "input.key",
        "input.scroll",
        "input.click_at",
        "eval.run",
        "wait.for",
        "events.subscribe",
        "events.unsubscribe",
    ];
    for (file, msg) in wire_messages() {
        let Some(m) = msg.get("method").and_then(Value::as_str) else {
            continue;
        };
        if m.starts_with("event.") {
            continue; // notifications serveur — couvertes par Event
        }
        assert!(known.contains(&m), "{file}: méthode inconnue {m}");
    }
}

/// Les erreurs golden portent des codes EAP/JSON-RPC connus.
#[test]
fn golden_error_codes_known() {
    for (file, msg) in wire_messages() {
        let Some(err) = msg.get("error") else {
            continue;
        };
        let code = err["code"].as_i64().expect("code entier");
        assert!(
            ErrorCode::try_from(code).is_ok(),
            "{file}: code inconnu {code}"
        );
    }
}

/// Roundtrip binaire : encode → decode restitue id/kind/payload.
#[test]
fn binary_frame_roundtrip() {
    let payload = b"\x89PNG fake bytes";
    let mut buf = Vec::new();
    proto::binary::encode(14, proto::binary::Kind::PngFrame, payload, &mut buf);
    let (id, kind, body) = proto::binary::decode(&buf).expect("décodage");
    assert_eq!(id, 14);
    assert_eq!(kind, proto::binary::Kind::PngFrame);
    assert_eq!(body, payload);
    // Trames fautives → None, jamais de panic.
    assert!(proto::binary::decode(&buf[..4]).is_none());
    assert!(proto::binary::decode(&[]).is_none());
}

/// `session.open` minimal (params vides) est légal — profil par
/// défaut restrictif.
#[test]
fn session_open_defaults_restrictive() {
    let req = json!({
        "jsonrpc": "2.0", "id": 1,
        "method": "session.open", "params": {}
    });
    let Incoming::Request { method, .. } = serde_json::from_value(req).expect("parse") else {
        panic!("requête attendue");
    };
    let Method::SessionOpen(p) = method else {
        panic!("session.open attendu");
    };
    assert!(p.capabilities.is_none());
    assert!(p.profile.is_none());
}

/// `params` absent entièrement est légal pour les méthodes sans
/// arguments — les deux formes (`"params":{}` et rien) parsent.
#[test]
fn params_absent_accepted() {
    for m in ["tab.open", "session.ping", "tab.list"] {
        let req = json!({"jsonrpc": "2.0", "id": 1, "method": m});
        let parsed: Result<Incoming, _> = serde_json::from_value(req);
        assert!(parsed.is_ok(), "{m} sans params doit parser : {parsed:?}");
    }
}

/// `profile.tabs` accepte `"all"` et `[ids]` — les deux formes fil.
#[test]
fn tab_scope_wire_forms() {
    let req = |tabs: Value| {
        json!({
            "jsonrpc": "2.0", "id": 1, "method": "session.open",
            "params": {"profile": {"input": true, "tabs": tabs}}
        })
    };
    let Incoming::Request { method, .. } = serde_json::from_value(req(json!("all"))).expect("all")
    else {
        panic!()
    };
    let Method::SessionOpen(p) = method else {
        panic!()
    };
    assert_eq!(p.profile.unwrap().tabs, proto::TabScope::all());

    let Incoming::Request { method, .. } = serde_json::from_value(req(json!([3, 7]))).expect("ids")
    else {
        panic!()
    };
    let Method::SessionOpen(p) = method else {
        panic!()
    };
    assert_eq!(p.profile.unwrap().tabs, proto::TabScope::Only(vec![3, 7]));
}
