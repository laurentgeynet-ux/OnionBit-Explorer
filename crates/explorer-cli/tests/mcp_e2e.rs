// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Test de bout en bout de l'adaptateur MCP↔EAP : un vrai
//! processus `explorer-cli mcp` (stdio pipé) contre le
//! routeur agent sur listener éphémère — initialize,
//! tools/list, `tab_new`/`navigate`/`select`/`click`, le tout en
//! passant par le protocole canonique.

use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use explorer_core::Session;
use explorer_layout::font::{FontRequest, FontSource};
use explorer_net::error::NetResult;
use explorer_net::http::{HttpTransport, Request, Response};
use explorer_test_support::fixture_ttf;
use serde_json::{json, Value};

struct FixtureFont;
impl FontSource for FixtureFont {
    fn font_data(&self, _req: &FontRequest) -> Option<Vec<u8>> {
        Some(fixture_ttf())
    }
}

const PAGE: &str = "<!doctype html><html><head><title>t</title></head><body>\
    <a href=\"/next\">go</a><button id=\"b\">ok</button>\
    <input type=\"text\" id=\"f\"></body></html>";

struct FakeTransport;
#[async_trait]
impl HttpTransport for FakeTransport {
    async fn fetch(&self, _req: &Request) -> NetResult<Response> {
        Ok(Response {
            status: 200,
            reason: "fake".to_string(),
            headers: vec![],
            body: PAGE.as_bytes().to_vec(),
            reused_connection: false,
            resolved_url: None,
            from_cache: false,
        })
    }
}

/// Le binaire `mcp` en sous-process, stdio pipé — lecture
/// dans un thread (les pipes Windows n'ont pas de timeout).
struct Mcp {
    child: Child,
    stdin: ChildStdin,
    lines: mpsc::Receiver<String>,
    next: AtomicU64,
}

impl Mcp {
    fn spawn(port: u16) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_explorer-cli"))
            .arg("mcp")
            .arg(port.to_string())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn explorer-cli mcp");
        let stdin = child.stdin.take().expect("stdin");
        let mut stdout = BufReader::new(child.stdout.take().expect("stdout"));
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let mut line = String::new();
            while stdout.read_line(&mut line).is_ok() {
                let trimmed = line.trim_end().to_string();
                line.clear();
                if trimmed.is_empty() {
                    continue;
                }
                if tx.send(trimmed).is_err() {
                    return;
                }
            }
        });
        Self {
            child,
            stdin,
            lines: rx,
            next: AtomicU64::new(1),
        }
    }

    /// Envoie une requête et attend la réponse JSON-RPC (le
    /// pont n'émet pas de notification — une ligne suffit).
    fn call(&mut self, method: &str, params: &Value) -> Value {
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        let req = json!({"jsonrpc":"2.0","id":id,"method":method,"params":params});
        writeln!(self.stdin, "{req}").expect("write");
        self.stdin.flush().expect("flush");
        let line = self
            .lines
            .recv_timeout(Duration::from_secs(30))
            .expect("réponse mcp");
        serde_json::from_str(&line).expect("json")
    }

    fn tool(&mut self, name: &str, args: &Value) -> Value {
        self.call("tools/call", &json!({"name": name, "arguments": args}))
    }
}

impl Drop for Mcp {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[tokio::test]
async fn mcp_pilote_eap() {
    let session = Arc::new(tokio::sync::Mutex::new(Session::new(
        Arc::new(FakeTransport),
        Arc::new(FixtureFont),
    )));
    let app = explorer_agent::agent_router(session, None);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let port = listener.local_addr().expect("addr").port();
    let server = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });

    let mcp = tokio::task::spawn_blocking(move || {
        let mut mcp = Mcp::spawn(port);
        // initialize → version négociée.
        let r = mcp.call(
            "initialize",
            &json!({"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "t", "version": "0"}}),
        );
        assert_eq!(r["result"]["serverInfo"]["name"], "explorer-eap", "{r}");
        // tools/list → la surface agent.
        let r = mcp.call("tools/list", &json!({}));
        let tools = r["result"]["tools"].as_array().expect("tools");
        for name in ["navigate", "snapshot", "select", "click", "type", "wait_for", "screenshot"] {
            assert!(tools.iter().any(|t| t["name"] == name), "outil {name} absent");
        }
        // tab_new → navigate → snapshot sémantique.
        let r = mcp.tool("tab_new", &json!({}));
        let tab = r["result"]["content"][0]["text"]
            .as_str()
            .and_then(|t| serde_json::from_str::<Value>(t).ok())
            .and_then(|v| v["tab"].as_u64())
            .expect("tab id");
        let r = mcp.tool("navigate", &json!({"tab": tab, "url": "http://t.local/x"}));
        assert!(r["result"]["isError"].is_null(), "{r}");
        let r = mcp.tool("snapshot", &json!({"tab": tab}));
        let text = r["result"]["content"][0]["text"].as_str().expect("text");
        let snap: Value = serde_json::from_str(text).expect("snapshot json");
        assert!(snap["nodes"]
            .as_array()
            .expect("nodes")
            .iter()
            .any(|n| n["role"] == "link" && n["name"] == "go"));
        // select → ref stable → click → navigated.
        let r = mcp.tool("select", &json!({"tab": tab, "selector": "a"}));
        let text = r["result"]["content"][0]["text"].as_str().expect("text");
        let sel: Value = serde_json::from_str(text).expect("select json");
        let elem_ref = sel["refs"][0].as_str().expect("ref").to_string();
        let r = mcp.tool("click", &json!({"tab": tab, "ref": elem_ref}));
        let text = r["result"]["content"][0]["text"].as_str().expect("text");
        let click: Value = serde_json::from_str(text).expect("click json");
        assert_eq!(click["result"], "navigated", "{r}");
        mcp
    })
    .await
    .expect("join");
    drop(mcp);
    server.abort();
}
