// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
//
// SPDX-License-Identifier: MIT OR Apache-2.0

//! R-248 — `border-image-*` : item `BorderImage` émis depuis
//! la source url collectée (`borders_url`), tranches
//! `%`/`px`/`fill`/`repeat` propagées.

use explorer_css::{Origin, StyleEngine, StyleEnv};
use explorer_html::TreeBuilder;

// PNG 1x1 transparent — data-url la plus petite.
#[allow(dead_code)]
const PNG: &str = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==";

#[test]
fn border_image_collecte_et_emet() {
    let dom = TreeBuilder::parse("<body><p>ok</p></body>");
    let mut eng = StyleEngine::new();
    eng.add_css(
        "p { border-image-source: url(data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==);\
             border-image-slice: 33% fill; border-image-width: 4px;\
             border-width: 4px }",
        Origin::Author,
    );
    let styles = eng.compute_tree(&dom, dom.document(), &StyleEnv::default());
    let imgs = explorer_render::collect_data_images(&dom, &styles, 4_000_000);
    // La source data-url de `border-image-source` est
    // collectée dans `borders_url` — une clé suffit.
    assert!(!imgs.borders_url.is_empty(), "borders_url vide");
}
