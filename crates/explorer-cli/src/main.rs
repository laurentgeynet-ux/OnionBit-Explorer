// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! `explorer-cli` — binaire de développement et debug.
//!
//! Commandes :
//! - `dump <fichier|url>` : parse le HTML et affiche le DOM en
//!   dump indenté (format html5lib-tests) — jalon J2.
//! - `fetch <url>` : télécharge et affiche le corps décodé.
//! - `layout <fichier|url>` : pipeline complet → wireframe de
//!   l'arbre de boîtes (dump wireframe, étape 17).

#![forbid(unsafe_code)]

use std::process::ExitCode;
use std::sync::Arc;

use explorer_html::TreeBuilder;
use explorer_net::decode::DecodingTransport;
use explorer_net::h1::HttpConfig;
use explorer_net::h2::AutoClient;
use explorer_net::http::{HttpTransport, Request};
use explorer_net::policy::RedirectTransport;
use explorer_net::tls::TlsConnector;
use explorer_net::url::Url;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    match args.as_slice() {
        [_, cmd, arg] if cmd == "dump" => dump(arg),
        [_, cmd, arg] if cmd == "fetch" => fetch(arg),
        [_, cmd, arg] if cmd == "layout" => layout(arg),
        _ => {
            eprintln!("usage : explorer-cli dump <fichier|url>");
            eprintln!("        explorer-cli fetch <url>");
            eprintln!("        explorer-cli layout <fichier|url>");
            ExitCode::FAILURE
        }
    }
}

/// Lit une source : fichier local (chemin ou `file:`) ou URL
/// `http(s)` via la pile `explorer-net` complète.
async fn read_source(src: &str) -> Result<Vec<u8>, String> {
    if src.starts_with("http://") || src.starts_with("https://") {
        let client: Arc<dyn HttpTransport> = {
            let tls = Arc::new(TlsConnector::web_roots());
            let h1: Arc<dyn HttpTransport> = Arc::new(AutoClient::new(tls, HttpConfig::default()));
            let decoded: Arc<dyn HttpTransport> =
                Arc::new(DecodingTransport::wrap(h1, 16 * 1024 * 1024));
            Arc::new(RedirectTransport::wrap(decoded, 8))
        };
        let url = Url::parse(src).map_err(|e| format!("url : {e}"))?;
        let resp = client
            .fetch(&Request::get(url))
            .await
            .map_err(|e| format!("fetch : {e}"))?;
        if !(200..300).contains(&resp.status) {
            return Err(format!("HTTP {}", resp.status));
        }
        Ok(resp.body)
    } else {
        let path = src.strip_prefix("file://").unwrap_or(src);
        std::fs::read(path).map_err(|e| format!("{path} : {e}"))
    }
}

#[tokio::main(flavor = "current_thread")]
async fn dump(src: &str) -> ExitCode {
    match read_source(src).await {
        Ok(bytes) => {
            let (html, enc) = explorer_html::encoding::decode_sniffed(&bytes);
            eprintln!("[encodage : {enc:?}]");
            let dom = TreeBuilder::parse(&html);
            print!("{}", explorer_html::debug::dump(&dom));
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("erreur : {e}");
            ExitCode::FAILURE
        }
    }
}

/// Pipeline fetch → parse → style → boîtes → wireframe.
#[tokio::main(flavor = "current_thread")]
async fn layout(src: &str) -> ExitCode {
    match read_source(src).await {
        Ok(bytes) => {
            let (html, enc) = explorer_html::encoding::decode_sniffed(&bytes);
            eprintln!("[encodage : {enc:?}]");
            let dom = TreeBuilder::parse(&html);
            let mut eng = explorer_css::StyleEngine::new();
            eng.add_css(&collect_style_css(&dom), explorer_css::Origin::Author);
            let styles = eng.compute_tree(&dom, dom.document(), &explorer_css::StyleEnv::default());
            let Some(root_el) = dom
                .children(dom.document())
                .into_iter()
                .flatten()
                .find(|&c| dom.element(c).is_ok())
            else {
                eprintln!("aucun élément sous le document");
                return ExitCode::FAILURE;
            };
            if let Some(bx) = explorer_layout::build_box_tree(&dom, root_el, &styles) {
                print!("{}", bx.dump(&dom));
                ExitCode::SUCCESS
            } else {
                eprintln!("racine display:none — rien à peindre");
                ExitCode::FAILURE
            }
        }
        Err(e) => {
            eprintln!("erreur : {e}");
            ExitCode::FAILURE
        }
    }
}

/// Concatène le contenu des `<style>` du document (feuilles
/// liées : hors périmètre de l'étape 17).
fn collect_style_css(dom: &explorer_dom::Dom) -> String {
    let mut css = String::new();
    let mut queue = vec![dom.document()];
    while let Some(n) = queue.pop() {
        if dom.tag_name(n).is_ok_and(|t| t == "style") {
            if let Ok(t) = dom.text_content(n) {
                css.push_str(&t);
                css.push('\n');
            }
        }
        if let Ok(ch) = dom.children(n) {
            queue.extend(ch.iter().rev());
        }
    }
    css
}

#[tokio::main(flavor = "current_thread")]
async fn fetch(src: &str) -> ExitCode {
    match read_source(src).await {
        Ok(bytes) => {
            use std::io::Write as _;
            let _ = std::io::stdout().write_all(&bytes);
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("erreur : {e}");
            ExitCode::FAILURE
        }
    }
}
