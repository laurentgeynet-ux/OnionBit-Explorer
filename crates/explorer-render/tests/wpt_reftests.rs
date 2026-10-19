// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Reftests **WPT ciblés** — étape 48 (ADR-0010).
//!
//! Mécanisme identique aux navigateurs : chaque test déclare
//! `<link rel="match">` (rendu identique) ou `rel="mismatch"`
//! (rendu différent) ; on rasterise test et référence dans le
//! même viewport 800×600 et on compare les frames **pixel à
//! pixel**. La fonte de test **Ahem** (métriques 1em×1em,
//! extraite du WPT) sert tous les textes — c'est ce qui rend
//! la comparaison pixel-exacte légitime.
//!
//! Corpus : `tests/data/wpt/` extrait par
//! `scripts/fetch_wpt.ps1` depuis `web-platform-tests/wpt`
//! (@ master 2026-02) — jamais édité à la main. Un cas
//! divergent entre dans `wpt_skip.txt` (versionné, raisonné).
//!
//! Le cliquet [`WPT_BASELINE`] monte au fil des progrès moteur.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock};

use explorer_css::{Origin, StyleEngine, StyleEnv};
use explorer_dom::{Dom, NodeId};
use explorer_html::TreeBuilder;
use explorer_layout::{build_box_tree_sized, layout_tree, BlockCtx, FontRequest, FontSource};
use explorer_render::{
    build_display_list_full, decode_image, rasterize, Frame, ImageTable, PaintImages,
};

/// Viewport reftest standard du WPT.
const VIEW_W: f32 = 800.0;
const VIEW_H: f32 = 600.0;

/// Cliquet : score minimal attendu — mesuré 2026-02 à 17/40
/// (42 %) sur la première extraction ; à monter à chaque
/// progrès float/BFC/marges.
const WPT_BASELINE: usize = 38;

/// Tolérance AA : pixels dont le delta maximal de canal est
/// ≤ [`FUZZY_CHANNEL`] comptent « doux » ; au-delà « dur ».
const FUZZY_CHANNEL: u8 = 16;
/// Nombre maximal de pixels « doux » tolérés (800×600).
const FUZZY_MAX: usize = 512;

/// `(durs, doux)` — pixels divergents par gravité.
fn diff_stats(a: &Frame, b: &Frame) -> (usize, usize) {
    if a.width != b.width || a.height != b.height {
        return (usize::MAX, 0);
    }
    let mut hard = 0;
    let mut soft = 0;
    for (pa, pb) in a.pixels.chunks(4).zip(b.pixels.chunks(4)) {
        let d = pa
            .iter()
            .zip(pb)
            .map(|(x, y)| x.abs_diff(*y))
            .max()
            .unwrap_or(0);
        if d > FUZZY_CHANNEL {
            hard += 1;
        } else if d > 0 {
            soft += 1;
        }
    }
    (hard, soft)
}

/// Fonte de test WPT — métriques uniformes 1em×1em.
static AHEM: LazyLock<Vec<u8>> =
    LazyLock::new(|| std::fs::read(data_dir().join("ahem.ttf")).expect("ahem.ttf"));

fn data_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/wpt")
}

struct Ahem;
impl FontSource for Ahem {
    fn font_data(&self, _req: &FontRequest) -> Option<Vec<u8>> {
        Some(AHEM.clone())
    }
}

fn ctx() -> BlockCtx<'static> {
    static SRC: Ahem = Ahem;
    BlockCtx {
        fonts: Some(&SRC),
        ..BlockCtx::default()
    }
}

/// Retire le prolog XML (`<?xml …?>`) que le tokenizer HTML
/// traiterait en tag-soup parasite.
fn strip_xml_prolog(src: &str) -> &str {
    let s = src.trim_start_matches('\u{feff}').trim_start();
    s.strip_prefix("<?").map_or(s, |rest| {
        rest.find("?>").map_or(s, |i| rest[i + 2..].trim_start())
    })
}

/// CSS des `<style>` de la page (ordre document).
fn collect_css(dom: &Dom) -> String {
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

/// Premier `<link rel="match|mismatch">` — `(kind, href)`.
fn ref_link(dom: &Dom) -> Option<(&'static str, String)> {
    let mut queue = vec![dom.document()];
    while let Some(n) = queue.pop() {
        if dom.tag_name(n).is_ok_and(|t| t == "link") {
            let rel = dom.attribute(n, "rel").ok().flatten().unwrap_or_default();
            let href = dom.attribute(n, "href").ok().flatten();
            let kind = match rel.as_str() {
                "match" => "match",
                "mismatch" => "mismatch",
                _ => "",
            };
            if let (true, Some(href)) = (!kind.is_empty(), href) {
                return Some((kind, href));
            }
        }
        if let Ok(ch) = dom.children(n) {
            queue.extend(ch.iter().rev());
        }
    }
    None
}

/// Borne de décodage d'une image du corpus (pixels décodés).
const IMG_MAX_PX: u32 = 1_000_000;

/// `<img src>` → `(tailles intrinsèques, pixels décodés)`,
/// `src` résolu relativement au fichier source (`support/`…).
#[allow(clippy::cast_precision_loss)] // pixels image ≪ 2^23
fn collect_images(dom: &Dom, file_dir: &Path) -> (HashMap<NodeId, (f32, f32)>, ImageTable) {
    let mut sizes = HashMap::new();
    let mut images = ImageTable::new();
    let mut queue = vec![dom.document()];
    while let Some(n) = queue.pop() {
        if dom.tag_name(n).is_ok_and(|t| t == "img") {
            if let Some(src) = dom.attribute(n, "src").ok().flatten() {
                let path = file_dir.join(&src);
                if let Ok(bytes) = std::fs::read(&path) {
                    if let Some(img) = decode_image(&bytes, IMG_MAX_PX) {
                        sizes.insert(n, (img.width as f32, img.height as f32));
                        images.insert(n, Arc::new(img));
                    }
                }
            }
        }
        if let Ok(ch) = dom.children(n) {
            queue.extend(ch.iter().rev());
        }
    }
    (sizes, images)
}

/// `url(...)` → chemin relatif (guillemets optionnels).
fn bg_url(raw: &str) -> Option<String> {
    let inner = raw.trim().strip_prefix("url(")?.strip_suffix(')')?;
    Some(
        inner
            .trim()
            .trim_matches('"')
            .trim_matches('\'')
            .to_string(),
    )
}

/// `background-image` des éléments → bitmaps décodés,
/// résolus relativement au fichier source.
fn collect_backgrounds(dom: &Dom, styles: &explorer_css::StyleMap, file_dir: &Path) -> ImageTable {
    let mut out = ImageTable::new();
    let mut queue = vec![dom.document()];
    while let Some(n) = queue.pop() {
        if dom.element(n).is_ok() {
            if let Some(url) = styles
                .get(n)
                .and_then(|st| st.get("background-image"))
                .and_then(bg_url)
            {
                if let Ok(bytes) = std::fs::read(file_dir.join(&url)) {
                    if let Some(img) = decode_image(&bytes, IMG_MAX_PX) {
                        out.insert(n, Arc::new(img));
                    }
                }
            }
        }
        if let Ok(ch) = dom.children(n) {
            queue.extend(ch.iter().rev());
        }
    }
    out
}

/// Pipeline complet : HTML → cascade (UA + `<style>`) →
/// layout → display list → raster RGBA. `file_dir` = dossier
/// du fichier source (résolution des `src`/`url()` d'images).
fn render(src: &str, file_dir: &Path) -> Option<Frame> {
    let dom = TreeBuilder::parse(strip_xml_prolog(src));
    let mut eng = StyleEngine::new();
    eng.add_css(&collect_css(&dom), Origin::Author);
    let styles = eng.compute_tree(&dom, dom.document(), &StyleEnv::default());
    let root = dom
        .children(dom.document())
        .into_iter()
        .flatten()
        .find(|&c| dom.element(c).is_ok())?;
    let (sizes, replaced) = collect_images(&dom, file_dir);
    let images = PaintImages {
        replaced,
        backgrounds: collect_backgrounds(&dom, &styles, file_dir),
        svgs: explorer_render::ImageTable::new(),
        canvases: explorer_render::CanvasTable::new(),
    };
    let mut bx = build_box_tree_sized(&dom, root, &styles, &sizes)?;
    layout_tree(&mut bx, VIEW_W, &ctx());
    let list = build_display_list_full(&bx, VIEW_W, VIEW_H, 0.0, &images);
    rasterize(&list, &Ahem, 1.0)
}

/// `href` de la référence résolu relativement au test —
/// gère `./x` et `../reference/x` (un seul niveau de `..`
/// observé dans ce corpus).
fn resolve(test_path: &Path, href: &str) -> PathBuf {
    let href = href.strip_prefix("./").unwrap_or(href);
    let mut dir = test_path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_default();
    let mut rel = href;
    while let Some(rest) = rel.strip_prefix("../") {
        dir.pop();
        rel = rest;
    }
    dir.join(rel)
}

/// Cas de la skip list : `nom` → raison documentée.
fn skip_list() -> HashSet<String> {
    let path = data_dir().join("wpt_skip.txt");
    let Ok(content) = std::fs::read_to_string(&path) else {
        return HashSet::new();
    };
    content
        .lines()
        .filter(|l| !l.trim().is_empty() && !l.trim_start().starts_with('#'))
        .filter_map(|l| l.split_whitespace().next().map(str::to_owned))
        .collect()
}

#[test]
fn wpt_reftests_pixel() {
    let dir = data_dir();
    let skip = skip_list();
    let manifest = std::fs::read_to_string(dir.join("manifest.txt")).expect("manifest.txt");
    let mut total = 0usize;
    let mut skipped = 0usize;
    let mut passed = 0usize;
    let mut failures: Vec<String> = Vec::new();
    let mut no_ref: Vec<String> = Vec::new();

    for line in manifest.lines() {
        let rel = line.trim();
        if rel.is_empty() || rel.starts_with('#') {
            continue;
        }
        let name = rel.to_string();
        if skip.contains(rel) {
            skipped += 1;
            continue;
        }
        total += 1;
        let test_path = dir.join(rel);
        let Ok(src) = std::fs::read_to_string(&test_path) else {
            failures.push(format!("{name}: illisible"));
            continue;
        };
        let test_dom = TreeBuilder::parse(strip_xml_prolog(&src));
        let Some((kind, href)) = ref_link(&test_dom) else {
            no_ref.push(name);
            continue;
        };
        let ref_path = resolve(&test_path, &href);
        let Ok(ref_src) = std::fs::read_to_string(&ref_path) else {
            failures.push(format!("{name}: ref {href} absente"));
            continue;
        };
        let test_dir = test_path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_default();
        let ref_dir = ref_path.parent().map(Path::to_path_buf).unwrap_or_default();
        let (Some(a), Some(b)) = (render(&src, &test_dir), render(&ref_src, &ref_dir)) else {
            failures.push(format!("{name}: rendu impossible"));
            continue;
        };
        // Comparaison « fuzzy » à la manière des harnais
        // reftest réels : l'antialiasing produit des deltas
        // de canal faibles sur les arêtes — on exige ZÉRO
        // pixel à écart > 16 et au plus `FUZZY_MAX` pixels
        // légèrement différents (~0,1 % de la frame).
        let (hard, soft) = diff_stats(&a, &b);
        let identical =
            a.width == b.width && a.height == b.height && hard == 0 && soft <= FUZZY_MAX;
        if identical == (kind == "match") {
            passed += 1;
        } else {
            let diff = hard * 4 + soft;
            failures.push(format!(
                "{name}: {kind}, {hard} px durs + {soft} px doux divergents"
            ));
            let _ = diff;
            // PNG de diagnostic sous `target/wpt_fail/` —
            // debug visuel, jamais assertif.
            if let (Some(pa), Some(pb)) = (a.encode_png(), b.encode_png()) {
                let out = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/wpt_fail");
                let _ = std::fs::create_dir_all(&out);
                let base = rel.rsplit('/').next().unwrap_or("t");
                let _ = std::fs::write(out.join(format!("{base}.test.png")), pa);
                let _ = std::fs::write(out.join(format!("{base}.ref.png")), pb);
            }
        }
    }

    let rate = passed * 100 / total.max(1);
    eprintln!("\n=== WPT reftests (CSS2 floats + normal-flow) ===");
    eprintln!("{passed}/{total} passent ({rate}%), {skipped} ignorés");
    if !no_ref.is_empty() {
        eprintln!("sans ref (exclus du score) : {}", no_ref.len());
        for f in &no_ref {
            eprintln!("  ? {f}");
        }
    }
    for f in &failures {
        eprintln!("  ✗ {f}");
    }
    assert!(
        passed >= WPT_BASELINE,
        "régression : {passed} < baseline {WPT_BASELINE}"
    );
    eprintln!("WPT_BASELINE courant = {WPT_BASELINE}, mesuré = {passed}");
}
