// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! R-13b : URLs `data:` — décodage pur (aucune I/O) et
//! collecte des images inline (`img`/`object`/`background`).

use explorer_css::{Origin, StyleEngine, StyleEnv};
use explorer_dom::Dom;
use explorer_html::TreeBuilder;
use explorer_render::{collect_data_images, data_url_bytes, decode_data_url};

/// PNG 1×1 jaune (celui d'Acid2 — base64 avec %2F/%3D
/// percent-encodés, la forme que produisent les auteurs).
const YELLOW_PX: &str = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAIAAACQd1PeAAAADElEQVR42mP4%2F58BAAT%2FAf9jgNErAAAAAElFTkSuQmCC";

fn styled(dom: &Dom, css: &str) -> explorer_css::StyleMap {
    let mut eng = StyleEngine::new();
    eng.add_css(css, Origin::Author);
    eng.compute_tree(dom, dom.document(), &StyleEnv::default())
}

#[test]
fn data_url_percent_plain() {
    let b = data_url_bytes("data:text/plain,hello%20world").expect("decode");
    assert_eq!(b, b"hello world");
}

#[test]
fn data_url_base64() {
    let b = data_url_bytes("data:application/octet-stream;base64,aGVsbG8=").expect("b64");
    assert_eq!(b, b"hello");
    // Padding absent toléré + espaces ignorés.
    let b2 = data_url_bytes("data:x;base64, aGVs bG8").expect("no-pad");
    assert_eq!(b2, b"hello");
}

#[test]
fn data_url_rejette_le_casse() {
    assert!(data_url_bytes("http://x").is_none());
    assert!(data_url_bytes("data:pas-de-virgule").is_none());
    assert!(data_url_bytes("data:;base64,").is_none());
    assert!(decode_data_url("data:image/png;base64,pas-une-image", 1000).is_none());
}

#[test]
fn data_url_image_png() {
    let img = decode_data_url(YELLOW_PX, 1_000_000).expect("png");
    assert_eq!((img.width, img.height), (1, 1));
    // Le pixel est jaune opaque (red+green ~255, blue ~0).
    assert_eq!(&img.rgba[..4], &[255, 255, 0, 255]);
}

/// R-34 : `<video poster="data:…">` suit le même canal
/// inline qu'`<img>` — l'affiche décodée sert à la fois la
/// peinture et la taille intrinsèque de la boîte média.
#[test]
fn collect_video_poster_replaced() {
    let dom = TreeBuilder::parse(&format!(
        "<html><body><video poster=\"{YELLOW_PX}\" id=v></video>\
         <audio id=a></audio></body></html>"
    ));
    let styles = styled(&dom, "");
    let imgs = collect_data_images(&dom, &styles, 1_000_000);
    assert_eq!(imgs.replaced.len(), 1, "seule l'affiche data: résout");
    let mut q = vec![dom.document()];
    let mut ok = false;
    while let Some(n) = q.pop() {
        if dom.attribute(n, "id").ok().flatten().as_deref() == Some("v") {
            let img = imgs.replaced.get(&n).expect("affiche du video");
            assert_eq!((img.width, img.height), (1, 1));
            ok = true;
        }
        if let Ok(c) = dom.children(n) {
            q.extend(c);
        }
    }
    assert!(ok, "nœud video trouvé et indexé");
}

#[test]
fn collect_img_et_object_replaced() {
    let dom = TreeBuilder::parse(&format!(
        "<html><body>\
         <img src=\"{YELLOW_PX}\" id=i>\
         <object data=\"{YELLOW_PX}\" id=o></object>\
         <img src=\"https://x/y.png\">\
         <object data=\"data:application/x-unknown,ERROR\"></object>\
         </body></html>"
    ));
    let styles = styled(&dom, "");
    let imgs = collect_data_images(&dom, &styles, 1_000_000);
    assert_eq!(imgs.replaced.len(), 2, "img+object data: résolus");
    // Le bon nœud est indexé.
    let mut q = vec![dom.document()];
    let mut seen_img = false;
    while let Some(n) = q.pop() {
        if let Ok(Some(id)) = dom.attribute(n, "id") {
            if id == "i" || id == "o" {
                assert!(imgs.replaced.contains_key(&n));
                seen_img = true;
            }
        }
        if let Ok(c) = dom.children(n) {
            q.extend(c);
        }
    }
    assert!(seen_img);
}

#[test]
fn collect_background_inline() {
    let dom = TreeBuilder::parse("<html><body><div class=x></div></body></html>");
    let styles = styled(&dom, &format!(".x {{ background: red url({YELLOW_PX}) }}"));
    let imgs = collect_data_images(&dom, &styles, 1_000_000);
    assert_eq!(imgs.backgrounds.len(), 1);
}

#[test]
fn data_urls_bornees_par_max_px() {
    // Une 2×2 décodée avec budget 1 px → refusée.
    let big = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAIAAAACCAIAAAD91JpzAAAABnRSTlMAAAAAAABupgeRAAAABmJLR0QA%2FwD%2FAP%2BgvaeTAAAAEUlEQVR42mP4%2F58BCv7%2FZwAAHfAD%2FabwPj4AAAAASUVORK5CYII%3D";
    assert!(decode_data_url(big, 1).is_none());
    assert!(decode_data_url(big, 1_000_000).is_some());
}
