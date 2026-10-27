// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Benchmarks du pipeline bout en bout (étape 39) :
//! `navigate` = fetch → decode → parse → cascade → layout →
//! display list → raster → PNG, sur transport simulé in-process
//! (aucune socket). `input_scroll` mesure le chemin repaint
//! (restyle + relayout + raster) sans le fetch.
//!
//! `cargo bench -p explorer-core` — résultats sous
//! `target/criterion/`.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use criterion::{criterion_group, criterion_main, Criterion};
use explorer_core::{InputEvent, Session, SessionConfig};
use explorer_layout::font::{FontRequest, FontSource};
use explorer_net::error::NetResult;
use explorer_net::http::{HttpTransport, Request, Response};
use explorer_test_support::fixture_ttf;

/// Fonte unique : la fixture TTF synthétique partagée.
struct FixtureFont;

impl FontSource for FixtureFont {
    fn font_data(&self, _req: &FontRequest) -> Option<Vec<u8>> {
        Some(fixture_ttf())
    }
}

/// Transport simulé : table `chemin → corps`.
struct FakeTransport {
    routes: Mutex<HashMap<String, Vec<u8>>>,
}

#[async_trait]
impl HttpTransport for FakeTransport {
    async fn fetch(&self, req: &Request) -> NetResult<Response> {
        let body = self
            .routes
            .lock()
            .ok()
            .and_then(|r| r.get(req.url.path()).cloned())
            .unwrap_or_else(|| b"missing".to_vec());
        Ok(Response {
            status: 200,
            reason: "fake".to_string(),
            headers: vec![],
            body,
            reused_connection: false,
            resolved_url: None,
        })
    }
}

/// Page représentative : typographie variée, liste, table,
/// formulaire — ~80 éléments, feuille inline modeste.
fn bench_page() -> String {
    let mut page = String::from(
        "<!doctype html><html><head><style>\
         body{font-family:sans-serif} h1,h2{margin:12px 0} \
         td,th{border:1px solid #999;padding:4px} \
         .note{color:#666;font-size:12px} li{margin:2px}\
         </style></head><body><h1>Titre principal</h1>",
    );
    for i in 0..12 {
        let _ = std::fmt::Write::write_fmt(
            &mut page,
            format_args!(
                "<h2>Section {i}</h2><p>Paragraphe {i} avec du \
                 <strong>texte fort</strong> et un \
                 <a href=\"/x\">lien</a>.</p>\
                 <ul><li>item {i}a</li><li>item {i}b \
                 <em>accentué</em></li></ul>\
                 <table><tr><th>A</th><th>B</th></tr>\
                 <tr><td>cell {i}1</td><td>cell {i}2</td></tr>\
                 </table><p class=\"note\">note de bas {i}</p>"
            ),
        );
    }
    page.push_str(
        "<form method=\"post\"><input name=\"n\" value=\"v\">\
         <input type=\"checkbox\" checked>\
         <select><option>o1</option></select>\
         <button>envoyer</button></form></body></html>",
    );
    page
}

fn bench_session() -> (Session, Arc<FakeTransport>) {
    let t = Arc::new(FakeTransport {
        routes: Mutex::new(HashMap::new()),
    });
    let s = Session::with_config(
        t.clone(),
        Arc::new(FixtureFont),
        SessionConfig {
            event_capacity: 16,
            viewport: (800.0, 600.0),
            scale: 1.0,
            js_ops: 5_000_000,
            max_images: 32,
            image_max_px: 16_000_000,
            max_sheets: 8,
            max_imports: 8,
            max_fonts: 16,
            max_scripts: 32,
            max_frames: 8,
            max_modules: 64,
            max_inflight: 6,
            max_script_net: 16,
            max_timer_jobs: 64,
            max_auto_navs: 64,
            paint_chunk_bytes: 32 * 1024,
            paint_interval_ms: 100,
            resync_gap_max_ms: 4_000,
            max_download_bytes: 64 * 1024 * 1024,
            bfcache_pages: 4,
            dom_journal_max_ops: 512,
            js_mem_limit_mib: 512,
            color_scheme: explorer_css::ColorScheme::default(),
            js_stall_cooldown: std::time::Duration::ZERO,
        },
    );
    (s, t)
}

fn pipeline(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap_or_else(|e| panic!("rt: {e}"));
    let (mut s, t) = bench_session();
    {
        let Ok(mut r) = t.routes.lock() else {
            return;
        };
        r.insert("/".to_string(), bench_page().into_bytes());
    }
    let tab = s.open_tab();
    rt.block_on(s.navigate(tab, "http://h.test/"))
        .unwrap_or_else(|e| panic!("navigate: {e}"));

    c.bench_function("pipeline_navigate", |b| {
        b.iter(|| rt.block_on(s.navigate(tab, "http://h.test/")));
    });
    c.bench_function("pipeline_scroll_repaint", |b| {
        b.iter(|| rt.block_on(s.handle_input(tab, InputEvent::Scroll { dy: 40.0 })));
    });
}

criterion_group!(benches, pipeline);
criterion_main!(benches);
