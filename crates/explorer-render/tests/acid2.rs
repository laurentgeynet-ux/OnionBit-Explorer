// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Jalon **Acid2** — étape 48 (ADR-0010).
//!
//! Acid2 (`acid2.acidtests.org`) est le test de conformité CSS2
//! le plus exigeant jamais publié : positionnement `fixed`,
//! `min/max-width/height`, sélecteurs d'attributs, `data:` URLs,
//! cascade de repli `<object>`, marges négatives, `clearance`,
//! `display: table`… Il est **très au-delà** du moteur actuel.
//!
//! Ce test fait deux choses honnêtes :
//!
//! 1. **Cliquet robustesse** : la page doit être rasterisée sans
//!    panic (« tolérant, jamais crash ») — c'est l'assertion.
//! 2. **Score informatif** : on cherche dans la frame la position
//!    où l'image de référence officielle (le visage, 168×168)
//!    s'aligne le mieux, et on rapporte le pourcentage de pixels
//!    de structure (non-blancs) de la référence qui coïncident.
//!    Pas d'assertion de score — un nombre remonté dans le
//!    rapport, à faire monter au fil des progrès moteur.
//!
//! Corpus : `tests/data/acid2.html` + `acid2-reference.png`
//! extraits de `http://acid2.acidtests.org/` (2026-02) par
//! `scripts/fetch_acid.ps1` — jamais édités à la main.

use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use explorer_css::{Origin, StyleEngine, StyleEnv};
use explorer_dom::Dom;
use explorer_html::TreeBuilder;
use explorer_layout::{build_box_tree_sized, layout_tree, BlockCtx, FontRequest, FontSource};
use explorer_render::{build_display_list_full, collect_data_images, rasterize, Frame};

/// Viewport haut : `#top { margin: 100em 3em 0 }` pousse le
/// visage ~1 400 px sous le haut de page ; 2 400 px le couvre
/// dans une mise en page correcte comme approximative.
const VIEW_W: f32 = 800.0;
const VIEW_H: f32 = 2400.0;
/// Budget pixels des `data:` images (borné — contenu hostile).
const IMG_MAX_PX: u32 = 1_000_000;

/// Fonte Ahem (métriques 1em×1em) — déterministe.
struct Ahem;
impl FontSource for Ahem {
    fn font_data(&self, _req: &FontRequest) -> Option<Vec<u8>> {
        Some(AHEM.clone())
    }
}

static AHEM: LazyLock<Vec<u8>> =
    LazyLock::new(|| std::fs::read(data_dir().join("wpt/ahem.ttf")).expect("ahem.ttf"));

fn data_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data")
}

/// CSS de la page : `<style>` inline + `<link rel="…stylesheet">`
/// dont le `href` est une `data:text/css` (R-13b — aucune I/O,
/// le corps est dans l'URL ; Acid2 l'utilise pour la feuille
/// « appendix » qui fait `.picture { background: none }`).
fn collect_css(dom: &Dom) -> String {
    let mut css = String::new();
    let mut queue = vec![dom.document()];
    while let Some(n) = queue.pop() {
        match dom.tag_name(n) {
            Ok("style") => {
                if let Ok(t) = dom.text_content(n) {
                    css.push_str(&t);
                    css.push('\n');
                }
            }
            Ok("link") => {
                if let Ok(Some(href)) = dom.attribute(n, "href") {
                    if href.trim_start().starts_with("data:text/css") {
                        if let Some(bytes) = explorer_render::data_url_bytes(href.trim()) {
                            css.push_str(&String::from_utf8_lossy(&bytes));
                            css.push('\n');
                        }
                    }
                }
            }
            _ => {}
        }
        if let Ok(ch) = dom.children(n) {
            queue.extend(ch.iter().rev());
        }
    }
    css
}

/// Pipeline complet : HTML → cascade → layout → raster.
/// `scroll` émule le défilement du viewport : le contenu en
/// flux remonte, les boîtes `position:fixed` et les fonds
/// `background-attachment:fixed` restent ancrés au viewport —
/// comme la capture de référence (le visage rempli l'écran,
/// la barre fixe recouvrant son sommet).
fn render(src: &str, scroll: f32) -> Option<Frame> {
    let dom = TreeBuilder::parse(src);
    let mut eng = StyleEngine::new();
    eng.add_css(&collect_css(&dom), Origin::Author);
    let styles = eng.compute_tree(&dom, dom.document(), &StyleEnv::default());
    let root = dom
        .children(dom.document())
        .into_iter()
        .flatten()
        .find(|&c| dom.element(c).is_ok())?;
    // Images `data:` inline (R-13b) — Acid2 n'a pas de
    // ressource réseau : tout arrive par URL inline.
    let images = collect_data_images(&dom, &styles, IMG_MAX_PX);
    #[allow(clippy::cast_precision_loss)]
    let sizes: std::collections::HashMap<_, (f32, f32)> = images
        .replaced
        .iter()
        .map(|(n, i)| (*n, (i.width as f32, i.height as f32)))
        .collect();
    let mut bx = build_box_tree_sized(&dom, root, &styles, &sizes)?;
    layout_tree(
        &mut bx,
        VIEW_W,
        &BlockCtx {
            fonts: Some(&Ahem as &dyn FontSource),
            ..BlockCtx::default()
        },
    );
    let list = build_display_list_full(&bx, VIEW_W, VIEW_H, scroll, &images);
    rasterize(&list, &Ahem, 1.0)
}

/// Pixel de frame démultiplié (`r,g,b`) — la frame est RGBA
/// prémultipliée sur fond blanc.
fn frame_rgb(f: &Frame, px: u32, py: u32) -> (u8, u8, u8) {
    let idx = ((py * f.width + px) * 4) as usize;
    let (red, green, blue, alpha) = (
        f.pixels[idx],
        f.pixels[idx + 1],
        f.pixels[idx + 2],
        f.pixels[idx + 3],
    );
    if alpha == 0 {
        return (255, 255, 255);
    }
    let un = |ch: u8| -> u8 {
        ((u32::from(ch) * 255 + u32::from(alpha) / 2) / u32::from(alpha)).min(255) as u8
    };
    (un(red), un(green), un(blue))
}

/// Échantillons de structure de la référence : `(x, y, r, g, b)`
/// non-blancs, un pixel sur quatre — précalculés une fois.
fn ref_samples(ref_rgb: &[u8]) -> Vec<(u32, u32, u8, u8, u8)> {
    let mut out = Vec::new();
    for ry in (0..168u32).step_by(4) {
        for rx in (0..168u32).step_by(4) {
            let idx = ((ry * 168 + rx) * 3) as usize;
            let (sr, sg, sb) = (ref_rgb[idx], ref_rgb[idx + 1], ref_rgb[idx + 2]);
            // Fond blanc : n'importe quoi matche — non discriminant.
            if !(sr > 240 && sg > 240 && sb > 240) {
                out.push((rx, ry, sr, sg, sb));
            }
        }
    }
    out
}

/// Pixels de `samples` qui coïncident avec la frame à
/// `(ox, oy)` — delta canal ≤ 16 (tolérance AA reftest).
/// `floor` : abandon précoce si le score ne peut plus
/// dépasser `floor` — accélère la recherche d'alignement.
fn align_score(
    f: &Frame,
    samples: &[(u32, u32, u8, u8, u8)],
    ox: u32,
    oy: u32,
    floor: usize,
) -> usize {
    let mut hit = 0usize;
    for (k, &(sx, sy, sr, sg, sb)) in samples.iter().enumerate() {
        if hit + (samples.len() - k) <= floor {
            return hit;
        }
        let (fr, fg, fb) = frame_rgb(f, ox + sx, oy + sy);
        if fr.abs_diff(sr).max(fg.abs_diff(sg)).max(fb.abs_diff(sb)) <= 16 {
            hit += 1;
        }
    }
    hit
}

/// Recherche du meilleur alignement — pas grossier (8 px)
/// puis raffinement ±8 px autour du maximum.
fn best_align(f: &Frame, samples: &[(u32, u32, u8, u8, u8)]) -> (usize, u32, u32) {
    let limit_y = f.height.saturating_sub(168);
    let limit_x = f.width.saturating_sub(168);
    let mut best = (0usize, 0u32, 0u32);
    for oy in (0..limit_y).step_by(8) {
        for ox in (0..limit_x).step_by(8) {
            let hit = align_score(f, samples, ox, oy, best.0);
            if hit > best.0 {
                best = (hit, ox, oy);
            }
        }
    }
    let (coarse_x, coarse_y) = (best.1, best.2);
    for oy in coarse_y.saturating_sub(8)..(coarse_y + 8).min(limit_y) {
        for ox in coarse_x.saturating_sub(8)..(coarse_x + 8).min(limit_x) {
            let hit = align_score(f, samples, ox, oy, best.0);
            if hit > best.0 {
                best = (hit, ox, oy);
            }
        }
    }
    best
}

/// `top:9em` de la barre `.picture p` (em = 12 px) — la
/// capture de référence a été faite document scrollé de
/// sorte que le visage commence sous la barre fixe.
const FIXED_BAR_TOP: f32 = 108.0;

#[test]
fn acid2_robustesse_et_score_informatif() {
    let dir = data_dir();
    let src = std::fs::read_to_string(dir.join("acid2.html")).expect("acid2.html");
    // Référence officielle (le visage attendu, 168×168).
    let ref_img = image::ImageReader::open(dir.join("acid2-reference.png"))
        .expect("acid2-reference.png")
        .decode()
        .expect("décodage référence")
        .to_rgb8();
    let samples = ref_samples(ref_img.as_raw());

    // Cliquet robustesse : rasterise sans panic, produit une frame.
    // Passe 1 : sans scroll — localise le visage (doc coords).
    let frame0 = render(&src, 0.0).expect("la page Acid2 doit produire une frame");
    let (_, _ox0, oy0) = best_align(&frame0, &samples);

    // Passe 2 : la référence a été capturée avec le document
    // scrollé — la barre `position:fixed` (top:9em) recouvre
    // le sommet du visage. On émule ce scroll : le contenu
    // remonte, la barre reste à 108 px du haut du viewport.
    #[allow(clippy::cast_precision_loss)]
    let scroll = (oy0 as f32 - FIXED_BAR_TOP).max(0.0);
    let frame = if scroll > 0.0 {
        render(&src, scroll).expect("frame scrollée")
    } else {
        frame0
    };

    // PNG de la frame complète pour inspection humaine.
    if let Some(png) = frame.encode_png() {
        let out = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target");
        let _ = std::fs::create_dir_all(&out);
        let _ = std::fs::write(out.join("acid2.png"), png);
    }

    let (hit, ox, oy) = best_align(&frame, &samples);
    #[allow(clippy::cast_precision_loss)]
    let rate = hit as f32 * 100.0 / samples.len().max(1) as f32;
    eprintln!(
        "\n=== Acid2 (informatif) ===\n\
         frame {VIEW_W}x{VIEW_H} scroll {scroll:.0} — meilleur alignement visage à \
         ({ox},{oy}) : {hit}/{} pixels de structure ({rate:.1}%)",
        samples.len(),
    );
}
