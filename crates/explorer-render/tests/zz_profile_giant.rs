// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Profil des phases PIPELINE sur un document géant réel
//! (reliure « apply post-script ~250 s », journal étape 112,
//! optimisé en R-82 — layout ~181 s → ~16 s mesuré sur
//! l'article de 2,2 Mo). Fontes réelles via `DirFontSource`
//! sur `%SystemRoot%/Fonts` comme la session live.
//! La fixture vit dans `target/probes/` (non versionnée) :
//! absente → la sonde se saute silencieusement.
//!
//! ```
//! cargo test -p explorer-render --test zz_profile_giant -- --ignored --nocapture
//! PROBE_HTML=<chemin> PROBE_FONTS=<dir> PROBE_NOFONTS=1
//! ```

use std::path::{Path, PathBuf};
use std::time::Instant;

use explorer_css::{Origin, StyleEngine, StyleEnv};
use explorer_dom::Dom;
use explorer_html::TreeBuilder;
use explorer_layout::{build_box_tree, layout_tree, BlockCtx};
use explorer_render::{
    build_display_list_full, rasterize_cached, DirFontSource, GlyphCache, PaintImages,
};

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

fn count_nodes(dom: &Dom) -> usize {
    let mut n = 0usize;
    let mut queue = vec![dom.document()];
    while let Some(x) = queue.pop() {
        n += 1;
        if let Ok(ch) = dom.children(x) {
            queue.extend(ch.iter().rev());
        }
    }
    n
}

#[test]
#[ignore = "profil à la demande — document géant réel"]
fn profile_giant() {
    let html_path = std::env::var("PROBE_HTML").map_or_else(
        |_| PathBuf::from("../../target/probes/brasil.html"),
        PathBuf::from,
    );
    let fonts_dir =
        std::env::var("PROBE_FONTS").unwrap_or_else(|_| r"C:\Windows\Fonts".to_string());
    let Ok(html) = std::fs::read_to_string(&html_path) else {
        eprintln!("fixture absente ({}) — sonde sautée", html_path.display());
        return;
    };
    eprintln!(
        "html           : {} octets ({})",
        html.len(),
        html_path.display()
    );

    let t = Instant::now();
    let dom = TreeBuilder::parse(&html);
    eprintln!(
        "parse          : {:?} ({} nœuds)",
        t.elapsed(),
        count_nodes(&dom)
    );

    let css = collect_css(&dom);
    eprintln!("css collecté   : {} octets", css.len());

    let t = Instant::now();
    let mut eng = StyleEngine::new();
    eng.add_css(&css, Origin::Author);
    let styles = eng.compute_tree(&dom, dom.document(), &StyleEnv::default());
    eprintln!("compute_styles : {:?}", t.elapsed());

    let root = dom
        .children(dom.document())
        .into_iter()
        .flatten()
        .find(|&c| dom.element(c).is_ok())
        .expect("racine élément");

    let no_fonts = std::env::var("PROBE_NOFONTS").is_ok();
    let fonts = if no_fonts {
        DirFontSource::new(Path::new("/nonexistent"))
    } else {
        let t = Instant::now();
        let f = DirFontSource::new(Path::new(&fonts_dir));
        eprintln!("scan fontes    : {:?}", t.elapsed());
        f
    };

    let ctx = BlockCtx {
        fonts: if no_fonts { None } else { Some(&fonts) },
        ..BlockCtx::default()
    };

    let t = Instant::now();
    let mut bx = build_box_tree(&dom, root, &styles).expect("boxtree");
    eprintln!("boxtree        : {:?}", t.elapsed());

    let t = Instant::now();
    layout_tree(&mut bx, 1024.0, &ctx);
    eprintln!("layout_tree    : {:?}", t.elapsed());

    // Coût d'un apply post-script à caches fonte chauds : la
    // session re-parse + recascade + reboîte, puis re-pose tout
    // (sauf si le relayout incrémental R-12 vise un sous-arbre —
    // bornes : aucun flottant, chemin bloc). Ici `ctx` réutilise
    // le `FontCache` exactement comme `PaintPayload.layout_font_cache`
    // (R-65) le fait entre applys.
    let t = Instant::now();
    let mut bx2 = build_box_tree(&dom, root, &styles).expect("boxtree#2");
    let t_box2 = t.elapsed();
    let t = Instant::now();
    layout_tree(&mut bx2, 1024.0, &ctx);
    eprintln!(
        "apply simulé   : boxtree {:?} + layout chaud {:?}",
        t_box2,
        t.elapsed()
    );

    let images = PaintImages::default();
    let t = Instant::now();
    let mut list = build_display_list_full(&bx, 1024.0, 768.0, 0.0, &images);
    if list.height > 16_384.0 {
        list.height = 16_384.0; // même cap que la session (MAX_FRAME_DIM)
    }
    eprintln!(
        "display list   : {:?} ({} items, h={})",
        t.elapsed(),
        list.items.len(),
        list.height
    );

    let mut gc = GlyphCache::new();
    let t = Instant::now();
    match rasterize_cached(&list, &fonts, 1.0, &mut gc) {
        Some(frame) => {
            eprintln!(
                "raster froid   : {:?} ({}×{})",
                t.elapsed(),
                frame.width,
                frame.height
            );
            let t = Instant::now();
            let _ = rasterize_cached(&list, &fonts, 1.0, &mut gc);
            eprintln!("raster chaud   : {:?}", t.elapsed());
            let t = Instant::now();
            let png = frame.encode_png().expect("png");
            eprintln!("encode_png     : {:?} ({} octets)", t.elapsed(), png.len());
        }
        None => eprintln!("raster         : None (frame refusée — {} px)", list.height),
    }
}
