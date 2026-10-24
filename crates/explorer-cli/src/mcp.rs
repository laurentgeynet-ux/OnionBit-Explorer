// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! `explorer-cli mcp` — serveur MCP stdio (JSON-RPC 2.0,
//! newline-delimited) qui wrappe l'API REST du daemon —
//! un agent vole le navigateur en natif : tabs, navigate,
//! frame, dom, layout, mem, console, input.
//! Hand-rolled (pas de crate MCP) : ~200 lignes de protocole
//! contre une dépendance jeune — le canal stdio porte la
//! confiance, pas d'auth.

use std::io::{BufRead, Read, Write};
use std::net::TcpStream;

use serde_json::{json, Value};

const PROTOCOL_VERSIONS: &[&str] = &["2025-06-18", "2025-03-26", "2024-11-05"];
const FALLBACK_VERSION: &str = "2024-11-05";

/// Boucle stdio : une trame JSON-RPC par ligne, jusqu'à EOF.
pub fn serve(port: u16) -> std::io::Result<u8> {
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    for line in stdin.lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let req: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(e) => {
                let err = json!({"jsonrpc":"2.0","id":null,
                    "error":{"code":-32700,"message":format!("parse: {e}")}});
                writeln!(out, "{err}")?;
                out.flush()?;
                continue;
            }
        };
        if let Some(resp) = handle(port, &req) {
            writeln!(out, "{resp}")?;
            out.flush()?;
        }
    }
    Ok(0)
}

/// Une requête → une réponse ; `None` pour les notifications.
fn handle(port: u16, req: &Value) -> Option<Value> {
    let method = req["method"].as_str()?;
    let id = req.get("id").cloned();
    let result = |r: Value| {
        id.clone()
            .map(|i| json!({"jsonrpc":"2.0","id":i,"result":r}))
    };
    let error = |code: i64, message: String| {
        id.clone()
            .map(|i| json!({"jsonrpc":"2.0","id":i,"error":{"code":code,"message":message}}))
    };
    match method {
        "initialize" => {
            let want = req["params"]["protocolVersion"].as_str().unwrap_or("");
            let v = if PROTOCOL_VERSIONS.contains(&want) {
                want
            } else {
                FALLBACK_VERSION
            };
            result(json!({
                "protocolVersion": v,
                "capabilities": {"tools": {"listChanged": false}},
                "serverInfo": {"name": "explorer-flight", "version": env!("CARGO_PKG_VERSION")},
            }))
        }
        "ping" => result(json!({})),
        m if m.starts_with("notifications/") => None,
        "tools/list" => result(json!({"tools": tools()})),
        "tools/call" => {
            let name = req["params"]["name"].as_str().unwrap_or("");
            let args = req["params"].get("arguments").cloned().unwrap_or(json!({}));
            Some(call_tool(port, id.as_ref(), name, &args))
        }
        _ => error(-32601, format!("méthode inconnue : {method}")),
    }
}

fn tools() -> Value {
    let t = |name: &str, desc: &str, schema: Value| json!({"name": name, "description": desc, "inputSchema": schema});
    let tab =
        || json!({"type":"object","properties":{"tab":{"type":"integer"}},"required":["tab"]});
    json!([
        t(
            "tab_new",
            "Ouvre un onglet → {id}",
            json!({"type":"object","properties":{}})
        ),
        t("tab_close", "Ferme l'onglet", tab()),
        t(
            "viewport",
            "Règle le viewport {tab,w,h}",
            json!({"type":"object","properties":{"tab":{"type":"integer"},"w":{"type":"integer"},"h":{"type":"integer"}},"required":["tab","w","h"]})
        ),
        t(
            "navigate",
            "Navigue {tab,url}",
            json!({"type":"object","properties":{"tab":{"type":"integer"},"url":{"type":"string"}},"required":["tab","url"]})
        ),
        t(
            "frame",
            "PNG de la frame → {path} (écrit le fichier)",
            json!({"type":"object","properties":{"tab":{"type":"integer"},"path":{"type":"string"}},"required":["tab","path"]})
        ),
        t("dom", "Dump DOM sérialisé", tab()),
        t("layout", "Dump layout (boîtes posées)", tab()),
        t("mem", "Rapport mémoire par réservoir", tab()),
        t(
            "console",
            "Erreurs pipeline enregistrées (sonde persistante)",
            tab()
        ),
        t(
            "input",
            "Input {tab,type:click|scroll|key|mousemove,x,y,dy,key}",
            json!({"type":"object","properties":{"tab":{"type":"integer"},"type":{"type":"string"},"x":{"type":"number"},"y":{"type":"number"},"dy":{"type":"number"},"key":{"type":"string"}},"required":["tab","type"]})
        ),
    ])
}

/// Dispatch outil → appel REST → `{content:[{type,text}]}`.
fn call_tool(port: u16, id: Option<&Value>, name: &str, args: &Value) -> Value {
    let resp = |v: Value| match id {
        Some(i) => json!({"jsonrpc":"2.0","id":i,"result":v}),
        None => v,
    };
    let tab = args["tab"].as_u64().unwrap_or(0);
    let text = |s: String| resp(json!({"content":[{"type":"text","text":s}]}));
    let outcome = match name {
        "tab_new" => rest(port, "POST", "/api/tabs", "{}"),
        "tab_close" => rest(port, "DELETE", &format!("/api/tabs/{tab}"), ""),
        "viewport" => rest(
            port,
            "POST",
            &format!("/api/tabs/{tab}/viewport"),
            &json!({"w": args["w"], "h": args["h"]}).to_string(),
        ),
        "navigate" => rest(
            port,
            "POST",
            &format!("/api/tabs/{tab}/navigate"),
            &json!({"url": args["url"]}).to_string(),
        ),
        "dom" => rest(port, "GET", &format!("/api/tabs/{tab}/dom"), ""),
        "layout" => rest(port, "GET", &format!("/api/tabs/{tab}/layout"), ""),
        "mem" => rest(port, "GET", &format!("/api/tabs/{tab}/mem"), ""),
        "console" => rest(port, "GET", &format!("/api/tabs/{tab}/console"), ""),
        "input" => rest(
            port,
            "POST",
            &format!("/api/tabs/{tab}/input"),
            &input_body(args),
        ),
        "frame" => return frame_tool(port, id, tab, args["path"].as_str().unwrap_or("frame.png")),
        _ => {
            return resp(
                json!({"content":[{"type":"text","text":format!("outil inconnu : {name}")}],"isError":true}),
            )
        }
    };
    match outcome {
        Ok(body) => text(body),
        Err(e) => resp(json!({"content":[{"type":"text","text":e}],"isError":true})),
    }
}

/// `input` : reconstruit le corps REST attendu par le daemon.
fn input_body(args: &Value) -> String {
    let mut b = serde_json::Map::new();
    b.insert("type".into(), args["type"].clone());
    for k in ["x", "y", "dy", "key"] {
        if args.get(k).is_some_and(|v| !v.is_null()) {
            b.insert(k.into(), args[k].clone());
        }
    }
    Value::Object(b).to_string()
}

/// `frame` : télécharge le PNG et l'écrit sur disque —
/// le chemin retourné (l'agent lit le fichier ensuite).
fn frame_tool(port: u16, id: Option<&Value>, tab: u64, path: &str) -> Value {
    let resp = |v: Value| match id {
        Some(i) => json!({"jsonrpc":"2.0","id":i,"result":v}),
        None => v,
    };
    match rest(port, "GET", &format!("/api/tabs/{tab}/frame"), "") {
        Ok(body) => match std::fs::write(path, body.as_bytes()) {
            Ok(()) => resp(json!({"content":[{"type":"text",
                "text":format!("frame écrite : {path} ({} octets)", body.len())}]})),
            Err(e) => resp(json!({"content":[{"type":"text",
                "text":format!("écriture {path} : {e}")}],"isError":true})),
        },
        Err(e) => resp(json!({"content":[{"type":"text","text":e}],"isError":true})),
    }
}

/// HTTP/1.1 loopback minimal — le daemon est plain-text local ;
/// `Connection: close` = lecture jusqu'à EOF, pas de chunked.
fn rest(port: u16, method: &str, path: &str, body: &str) -> Result<String, String> {
    let mut s = TcpStream::connect(("127.0.0.1", port))
        .map_err(|e| format!("daemon 127.0.0.1:{port} injoignable : {e}"))?;
    s.set_read_timeout(Some(std::time::Duration::from_secs(300)))
        .ok();
    let req = format!(
        "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\
         Content-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    );
    s.write_all(req.as_bytes()).map_err(|e| e.to_string())?;
    let mut buf = Vec::new();
    s.read_to_end(&mut buf).map_err(|e| e.to_string())?;
    let raw = String::from_utf8_lossy(&buf);
    // Sépare en-têtes/corps ; expose status + corps.
    let Some(pos) = raw.find("\r\n\r\n") else {
        return Err(format!(
            "réponse malformée : {}",
            &raw[..raw.len().min(120)]
        ));
    };
    let status = raw[..raw.find(' ').unwrap_or(0)].to_string()
        + &raw[raw.find(' ').unwrap_or(0)..raw.find("\r\n").unwrap_or(raw.len())];
    let body_str = raw[pos + 4..].to_string();
    if status.contains(" 4") || status.contains(" 5") {
        return Err(format!("{status} — {body_str}"));
    }
    Ok(body_str)
}
