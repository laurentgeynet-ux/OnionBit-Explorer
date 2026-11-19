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
use explorer_layout::{build_box_tree_sized_urls, layout_tree, BlockCtx, FontRequest, FontSource};
use explorer_render::{
    build_display_list_full, collect_data_images, decode_image, rasterize, Frame, ImageTable,
    PaintImages,
};

/// Viewport reftest standard du WPT.
const VIEW_W: f32 = 800.0;
const VIEW_H: f32 = 600.0;

/// Cliquet : score minimal attendu — mesuré 2026-02 à 17/40
/// (42 %) sur la première extraction ; 199/203 (98 %) après
/// la salve R-305 (counter-styles + css-lists). Les 4 échecs
/// comptés sont des trous moteur documentés (marqueur outside
/// alignement/image, `::marker` styling, strings rtl) —
/// à monter à chaque progrès moteur.
const WPT_BASELINE: usize = 202;

/// Tolérance AA : pixels dont le delta maximal de canal est
/// ≤ [`FUZZY_CHANNEL`] comptent « doux » ; au-delà « dur ».
const FUZZY_CHANNEL: u8 = 16;
/// Nombre maximal de pixels « doux » tolérés (800×600).
const FUZZY_MAX: usize = 512;

/// `(durs, doux)` — pixels divergents par gravité, évalués
/// sur la grille du **viewport** 800×600 : c'est la
/// sémantique capture-écran du harnais reftest réel — le
/// contenu sous la ligne de flottaison (frame plus haute que
/// `VIEW_H`) n'est jamais comparé, et un pixel absent dans
/// une frame plus courte compte comme blanc (diff si l'autre
/// a peint à cet endroit).
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
#[allow(clippy::many_single_char_names)] // grille de pixels — x/y/w/h courts
fn diff_stats(a: &Frame, b: &Frame) -> (usize, usize) {
    // VIEW_W/VIEW_H sont des entiers exacts en f32.
    let w = VIEW_W as usize;
    let h = VIEW_H as usize;
    let px = |f: &Frame, x: usize, y: usize| -> [u8; 4] {
        if x < f.width as usize && y < f.height as usize {
            let i = (y * f.width as usize + x) * 4;
            f.pixels
                .get(i..i + 4)
                .map_or([255; 4], |s| [s[0], s[1], s[2], s[3]])
        } else {
            [255; 4]
        }
    };
    let (mut hard, mut soft) = (0usize, 0usize);
    for y in 0..h {
        for x in 0..w {
            let pa = px(a, x, y);
            let pb = px(b, x, y);
            let delta = pa
                .iter()
                .zip(pb.iter())
                .map(|(p, q)| p.abs_diff(*q))
                .max()
                .unwrap_or(0);
            if delta > FUZZY_CHANNEL {
                hard += 1;
            } else if delta > 0 {
                soft += 1;
            }
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
    // R-32 : comme le harnais WPT réel, Ahem n'existe que
    // pour la famille « Ahem » — `serif`/`sans-serif` tombent
    // sur les métriques de repli (≈ fonte système). Servir
    // Ahem à toute requête faisait déborder « flt lef » à
    // 7×1em = 112 px → wrap fantôme (margin-collapse-039).
    fn font_data(&self, req: &FontRequest) -> Option<Vec<u8>> {
        req.families
            .iter()
            .any(|f| f.eq_ignore_ascii_case("ahem"))
            .then(|| AHEM.clone())
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

/// Éléments vides HTML — `/>` y est déjà correct en parsing
/// HTML, inutile de les toucher.
const VOID: &[&str] = &[
    "area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "param", "source",
    "track", "wbr",
];

/// Normalisation `.xhtml` → HTML : les fixtures XHTML sont
/// servies en XML par le WPT — `<div …/>` s'y ferme
/// réellement, alors que le parsing HTML ignore `/>` hors
/// éléments vides (la balise resterait ouverte et absorberait
/// tous ses « frères »). On réécrit `<t …/>` en `<t …></t>`
/// à la lecture — le corpus n'est pas modifié, on reproduit
/// juste la sémantique du type MIME.
#[allow(clippy::many_single_char_names)] // scanner d'octets — indices courts
fn normalize_xhtml(src: &str) -> String {
    let bytes = src.as_bytes();
    let mut out = String::with_capacity(src.len() + 64);
    let (mut i, mut last) = (0usize, 0usize);
    while i < bytes.len() {
        if bytes[i] == b'<' && i + 1 < bytes.len() && bytes[i + 1].is_ascii_alphabetic() {
            let mut j = i + 1;
            while j < bytes.len() && (bytes[j].is_ascii_alphanumeric() || bytes[j] == b'-') {
                j += 1;
            }
            let name = &src[i + 1..j];
            // Fin de balise : `>` hors guillemets d'attribut.
            let mut quote = None;
            let mut k = j;
            while k < bytes.len() {
                match (quote, bytes[k]) {
                    (Some(q), ch) if ch == q => quote = None,
                    (None, b'"' | b'\'') => quote = Some(bytes[k]),
                    (None, b'>') => break,
                    _ => {}
                }
                k += 1;
            }
            if k < bytes.len() && !VOID.contains(&name) {
                let tag = src[i..k].trim_end();
                if let Some(open) = tag.strip_suffix('/') {
                    out.push_str(&src[last..i]);
                    out.push_str(open);
                    out.push_str("></");
                    out.push_str(name);
                    out.push('>');
                    last = k + 1;
                    i = last;
                    continue;
                }
            }
        }
        i += 1;
    }
    out.push_str(&src[last..]);
    out
}

/// Lit une fixture : prolog XML retiré, et normalisation
/// `<t/>`→`<t></t>` pour les fichiers servis en XML (`.xhtml`,
/// `.xht`).
fn load_fixture(path: &Path) -> Option<String> {
    let src = std::fs::read_to_string(path).ok()?;
    let src = strip_xml_prolog(&src);
    match path.extension().and_then(|e| e.to_str()) {
        Some("xhtml" | "xht") => Some(normalize_xhtml(src)),
        _ => Some(src.to_string()),
    }
}

/// CSS des `<style>` et `<link rel="stylesheet">` de la page
/// (ordre document). Les feuilles liées sont lues depuis le
/// corpus local, résolues relativement au fichier source
/// (`support/…`) — le harnais réel les sert en HTTP ; ici
/// tout est fichier.
fn collect_css(dom: &Dom, file_dir: &Path) -> String {
    let mut css = String::new();
    let mut queue = vec![dom.document()];
    while let Some(n) = queue.pop() {
        if dom.tag_name(n).is_ok_and(|t| t == "style") {
            if let Ok(t) = dom.text_content(n) {
                css.push_str(&t);
                css.push('\n');
            }
        } else if dom.tag_name(n).is_ok_and(|t| t == "link") {
            let rel = dom.attribute(n, "rel").ok().flatten().unwrap_or_default();
            if rel
                .split_whitespace()
                .any(|t| t.eq_ignore_ascii_case("stylesheet"))
            {
                if let Some(href) = dom.attribute(n, "href").ok().flatten() {
                    if let Ok(sheet) = std::fs::read_to_string(resolve_in(file_dir, &href)) {
                        css.push_str(&sheet);
                        css.push('\n');
                    }
                }
            }
        }
        if let Ok(ch) = dom.children(n) {
            queue.extend(ch.iter().rev());
        }
    }
    css
}

/// `<meta name="flags" content="…">` — drapeaux WPT du test,
/// en minuscules (plusieurs tokens séparés par espaces).
fn meta_flags(dom: &Dom) -> Vec<String> {
    let mut out = Vec::new();
    let mut queue = vec![dom.document()];
    while let Some(n) = queue.pop() {
        if dom.tag_name(n).is_ok_and(|t| t == "meta") {
            let name = dom.attribute(n, "name").ok().flatten().unwrap_or_default();
            if name.eq_ignore_ascii_case("flags") {
                if let Some(content) = dom.attribute(n, "content").ok().flatten() {
                    out.extend(content.split_whitespace().map(str::to_ascii_lowercase));
                }
            }
        }
        if let Ok(ch) = dom.children(n) {
            queue.extend(ch.iter().rev());
        }
    }
    out
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

/// `background-image`/`list-style-image` des éléments →
/// bitmaps décodés, résolus relativement au fichier source.
/// Les `url(data:…)` sont couvertes en amont par
/// [`collect_data_images`] — même contrat que la production,
/// déséchappement CSS de la valeur calculée inclus.
fn collect_backgrounds(dom: &Dom, styles: &explorer_css::StyleMap, file_dir: &Path) -> ImageTable {
    let mut out = ImageTable::new();
    let mut queue = vec![dom.document()];
    while let Some(n) = queue.pop() {
        if dom.element(n).is_ok() {
            for prop in ["background-image", "list-style-image"] {
                let Some(url) = styles.get(n).and_then(|st| st.get(prop)).and_then(bg_url) else {
                    continue;
                };
                if url.starts_with("data:") {
                    continue;
                }
                if let Some(img) = std::fs::read(file_dir.join(&url))
                    .ok()
                    .and_then(|bytes| decode_image(&bytes, IMG_MAX_PX))
                {
                    out.insert(n, Arc::new(img));
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
#[allow(clippy::cast_precision_loss)] // pixels image ≪ 2^23
fn render(src: &str, file_dir: &Path, tag: &str) -> Option<Frame> {
    let dom = TreeBuilder::parse(strip_xml_prolog(src));
    let mut eng = StyleEngine::new();
    eng.add_css(&collect_css(&dom, file_dir), Origin::Author);
    let styles = eng.compute_tree(&dom, dom.document(), &StyleEnv::default());
    let root = dom
        .children(dom.document())
        .into_iter()
        .flatten()
        .find(|&c| dom.element(c).is_ok())?;
    let (mut sizes, mut replaced) = collect_images(&dom, file_dir);
    // `data:` inline — mêmes tables que la session
    // (`replaced` + leurs dimensions intrinsèques pour le
    // sizing, `backgrounds` dont `list-style-image`, R-306).
    let data_imgs = collect_data_images(&dom, &styles, IMG_MAX_PX);
    for (n, img) in &data_imgs.replaced {
        sizes
            .entry(*n)
            .or_insert((img.width as f32, img.height as f32));
    }
    replaced.extend(data_imgs.replaced);
    let mut backgrounds = collect_backgrounds(&dom, &styles, file_dir);
    backgrounds.extend(data_imgs.backgrounds);
    // R-306 — dims des `list-style-image` décodées → canal
    // `sizes` (même contrat que `image_sizes` en session) :
    // la ligne fantôme du marqueur dimensionne le
    // `list-item` même vide.
    for (n, img) in &backgrounds {
        if styles
            .get(*n)
            .and_then(|st| st.get("list-style-image"))
            .is_some_and(|v| v.contains("url("))
        {
            sizes
                .entry(*n)
                .or_insert((img.width as f32, img.height as f32));
        }
    }
    // R-308 — `content:url(data:)` de pseudo : dims →
    // `url_sizes` (canal `build_box_tree_sized_urls`),
    // bitmaps → `content_images`.
    let mut url_sizes: std::collections::HashMap<String, (f32, f32)> =
        std::collections::HashMap::new();
    for (u, img) in &data_imgs.content_images {
        url_sizes
            .entry(u.clone())
            .or_insert((img.width as f32, img.height as f32));
    }
    let images = PaintImages {
        replaced,
        backgrounds,
        background_layers: std::collections::HashMap::new(),
        svgs: explorer_render::ImageTable::new(),
        masks: explorer_render::ImageTable::new(),
        masks_url: std::collections::HashMap::new(),
        canvases: explorer_render::CanvasTable::new(),
        svg_sources: std::collections::HashMap::new(),
        borders_url: std::collections::HashMap::new(),
        content_images: data_imgs.content_images,
    };
    let mut bx = build_box_tree_sized_urls(&dom, root, &styles, &sizes, &url_sizes)?;
    layout_tree(&mut bx, VIEW_W, &ctx());
    if std::env::var("WPT_DUMP").is_ok_and(|f| tag.contains(&f)) {
        eprintln!("--- {tag}\n{}", bx.dump(&dom));
        let mut q = vec![root];
        while let Some(n) = q.pop() {
            if let Ok(el) = dom.element(n) {
                let st = styles.get(n);
                eprintln!(
                    "<{}> fs={:?} lh={:?} ff={:?}",
                    el.tag_name,
                    st.and_then(|s| s.get("font-size")),
                    st.and_then(|s| s.get("line-height")),
                    st.and_then(|s| s.get("font-family")),
                );
            }
            if let Ok(ch) = dom.children(n) {
                q.extend(ch.iter().rev());
            }
        }
    }
    let list = build_display_list_full(&bx, VIEW_W, VIEW_H, 0.0, &images);
    if std::env::var("WPT_DLIST").is_ok_and(|f| tag.contains(&f)) {
        eprintln!("--- display {tag}\n{}", list.dump());
    }
    rasterize(&list, &Ahem, 1.0)
}

/// `href` résolu relativement au dossier `dir` —
/// gère `./x` et `../reference/x` (un seul niveau de `..`
/// observé dans ce corpus) ; un href **absolu** `/css/…`
/// (R-32 : margin-collapse-clear-001 → floats-clear) se
/// résout depuis la racine du corpus.
fn resolve_in(dir: &Path, href: &str) -> PathBuf {
    if let Some(abs) = href.strip_prefix('/') {
        return data_dir().join(abs);
    }
    let href = href.strip_prefix("./").unwrap_or(href);
    let mut dir = dir.to_path_buf();
    let mut rel = href;
    while let Some(rest) = rel.strip_prefix("../") {
        dir.pop();
        rel = rest;
    }
    dir.join(rel)
}

/// `href` de la référence résolu relativement au test.
fn resolve(test_path: &Path, href: &str) -> PathBuf {
    resolve_in(
        &test_path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_default(),
        href,
    )
}

/// Flags `<meta name="flags">` que le harnais statique ne
/// peut pas exécuter — skip automatique (compté à part de
/// `wpt_skip.txt`, qui reste pour les divergences moteur ou
/// les limites non déclarées par la fixture) :
/// `dom`/`history`/`userstyle`/`speech` exigent JS ou un
/// agent non-statique, `interact`/`animated`/`paged` un
/// usage réel, `http`/`https` des en-têtes serveur,
/// `namespace`/`nonhtml`/`asis` un parseur XML ou un mode
/// de lecture brut que le harnais ne reproduit pas, `wait`
/// une attente d'événement asynchrone.
const UNRUNNABLE_FLAGS: &[&str] = &[
    "animated",
    "asis",
    "dom",
    "history",
    "http",
    "https",
    "interact",
    "namespace",
    "nonhtml",
    "paged",
    "speech",
    "userstyle",
    "wait",
];

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

/// Issue d'un cas du manifeste — `NoRef` et `FlagSkip` ne
/// comptent pas dans le score ; `Pass`/`Fail` oui.
enum Verdict {
    Pass,
    Fail(String),
    NoRef,
    FlagSkip(String),
}

/// Joue un cas du manifeste : fixture → classification par
/// `flags` → ref → rendu des deux côtés → comparaison
/// viewport. Les PNG de diagnostic des échecs atterrissent
/// sous `target/wpt_fail/` (debug visuel, jamais assertif).
fn run_case(dir: &Path, rel: &str) -> Verdict {
    let name = rel.to_string();
    let test_path = dir.join(rel);
    let Some(src) = load_fixture(&test_path) else {
        return Verdict::Fail(format!("{name}: illisible"));
    };
    let test_dom = TreeBuilder::parse(&src);
    if let Some(f) = meta_flags(&test_dom)
        .iter()
        .find(|f| UNRUNNABLE_FLAGS.contains(&f.as_str()))
    {
        return Verdict::FlagSkip(f.clone());
    }
    let Some((kind, href)) = ref_link(&test_dom) else {
        return Verdict::NoRef;
    };
    let ref_path = resolve(&test_path, &href);
    let Some(ref_src) = load_fixture(&ref_path) else {
        return Verdict::Fail(format!("{name}: ref {href} absente"));
    };
    let test_dir = test_path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_default();
    let ref_dir = ref_path.parent().map(Path::to_path_buf).unwrap_or_default();
    let (Some(a), Some(b)) = (
        render(&src, &test_dir, &name),
        render(&ref_src, &ref_dir, &format!("{name}(ref)")),
    ) else {
        return Verdict::Fail(format!("{name}: rendu impossible"));
    };
    // Comparaison « fuzzy » à la manière des harnais reftest
    // réels : l'antialiasing produit des deltas de canal
    // faibles sur les arêtes — on exige ZÉRO pixel à écart
    // > 16 et au plus `FUZZY_MAX` pixels légèrement
    // différents (~0,1 % de la frame).
    let (hard, soft) = diff_stats(&a, &b);
    let identical = hard == 0 && soft <= FUZZY_MAX;
    if identical == (kind == "match") {
        return Verdict::Pass;
    }
    if let (Some(pa), Some(pb)) = (a.encode_png(), b.encode_png()) {
        let out = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/wpt_fail");
        let _ = std::fs::create_dir_all(&out);
        let base = rel.rsplit('/').next().unwrap_or("t");
        let _ = std::fs::write(out.join(format!("{base}.test.png")), pa);
        let _ = std::fs::write(out.join(format!("{base}.ref.png")), pb);
    }
    Verdict::Fail(format!(
        "{name}: {kind}, {hard} px durs + {soft} px doux divergents"
    ))
}

#[test]
fn wpt_reftests_pixel() {
    let dir = data_dir();
    let skip = skip_list();
    let manifest = std::fs::read_to_string(dir.join("manifest.txt")).expect("manifest.txt");
    let mut total = 0usize;
    let mut skipped = 0usize;
    let mut flag_skipped: Vec<String> = Vec::new();
    let mut passed = 0usize;
    let mut failures: Vec<String> = Vec::new();
    let mut no_ref: Vec<String> = Vec::new();

    for line in manifest.lines() {
        let rel = line.trim();
        if rel.is_empty() || rel.starts_with('#') {
            continue;
        }
        // Ressources support listées dans le manifeste
        // (support/*.css…) pour que fetch_wpt.ps1 les
        // télécharge — pas des tests.
        if !matches!(
            rel.rsplit('.').next(),
            Some("html" | "htm" | "xht" | "xhtml" | "svg")
        ) {
            continue;
        }
        if skip.contains(rel) {
            skipped += 1;
            continue;
        }
        match run_case(&dir, rel) {
            Verdict::Pass => {
                total += 1;
                passed += 1;
            }
            Verdict::Fail(f) => {
                total += 1;
                failures.push(f);
            }
            Verdict::NoRef => no_ref.push(rel.to_string()),
            Verdict::FlagSkip(f) => flag_skipped.push(format!("{rel} [{f}]")),
        }
    }

    let rate = passed * 100 / total.max(1);
    eprintln!("\n=== WPT reftests (manifeste ciblé) ===");
    eprintln!(
        "{passed}/{total} passent ({rate}%), {skipped} ignorés (skip list), {} auto-skip (flags)",
        flag_skipped.len()
    );
    if !flag_skipped.is_empty() {
        eprintln!("auto-skip flags :");
        for f in &flag_skipped {
            eprintln!("  - {f}");
        }
    }
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
