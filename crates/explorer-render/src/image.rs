// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Images décodées — `DecodedImage` (RGBA) + table par nœud.
//!
//! Le décodage passe par le crate `image` (pur Rust — png, jpeg,
//! gif, webp, bmp, ico ; aucun `*-sys`). Le flux est borné
//! côté appelant : nombre d'images par page (`SessionConfig`)
//! et pixels décodés (`decode_image(max_px)`) — une image
//! hostile (dimension démesurée, format cassé) est refusée
//! proprement (`None` → l'appelant peint le repli `alt`).

use std::collections::HashMap;
use std::sync::Arc;

use explorer_dom::NodeId;

/// Une image décodée, prête à blitter (RGBA 8 bits/canal).
#[derive(Debug)]
pub struct DecodedImage {
    /// Largeur en pixels.
    pub width: u32,
    /// Hauteur en pixels.
    pub height: u32,
    /// Pixels RGBA, `width * height * 4` octets, ligne-major.
    pub rgba: Vec<u8>,
}

/// Images disponibles à la mise en page/peinture, par `NodeId`
/// de l'élément `<img>` source.
pub type ImageTable = HashMap<NodeId, Arc<DecodedImage>>;

/// Bitmaps de la page pour la peinture : éléments remplacés
/// (`<img>` — sert aussi le layout via leurs tailles) et fonds
/// (`background-image` — résolus et décodés par l'appelant,
/// clés par `NodeId` de l'élément porteur).
#[derive(Debug, Default)]
pub struct PaintImages {
    /// `<img>` décodés (`NodeId` → bitmap).
    pub replaced: ImageTable,
    /// `background-image: url(...)` décodés (`NodeId` → bitmap).
    pub backgrounds: ImageTable,
    /// `<svg>` inline rasterisés (`NodeId` → bitmap) — R-9,
    /// produits par [`crate::svg::collect_svg`].
    pub svgs: ImageTable,
    /// `mask-image: url(...)` décodés (`NodeId` de l'élément
    /// masqué → bitmap dont l'**alpha** sert de mask — R-52,
    /// motif « icône colorée » : `background-color` ne doit
    /// passer que là où le mask est opaque). Canal luminance
    /// ignoré — alpha seul, per spec `mask-mode` initial.
    pub masks: ImageTable,
    /// `mask-image` décodés **par url** — le texte `url(...)`
    /// brut du style calculé → bitmap. Complément de `masks`
    /// pour les porteurs sans `NodeId` (pseudos `::before`/
    /// `::after` — motif des icônes MDN) et pour les masks
    /// fetchés que l'appelant enregistre via
    /// [`PaintImages::register_mask`].
    pub masks_url: HashMap<String, Arc<DecodedImage>>,
    /// `<canvas>` : op-list 2D enregistrée par `explorer-jscore`
    /// (`NodeId` → `Vec<CanvasOp>` — R-11, contrat
    /// [`crate::canvas`]) ; rejouée vers un bitmap au moment
    /// de la génération de la display list.
    pub canvases: crate::canvas::CanvasTable,
}

impl PaintImages {
    /// Variante sans fonds ni SVG (compat de [`crate::ImageTable`]).
    #[must_use]
    pub fn from_replaced(replaced: ImageTable) -> Self {
        Self {
            replaced,
            backgrounds: ImageTable::new(),
            svgs: ImageTable::new(),
            masks: ImageTable::new(),
            masks_url: HashMap::new(),
            canvases: crate::canvas::CanvasTable::new(),
        }
    }

    /// Enregistre un `mask-image` **fetché** par l'appelant
    /// (couture réseau — symétrique des fonds `bg_decoded`).
    /// `url` = le texte `url(...)` **brut** tel qu'il apparaît
    /// dans le style calculé : c'est la clé de lookup à
    /// l'émission, y compris pour les boîtes pseudo sans
    /// `NodeId`. L'appelant gère l'alias url brute ↔ résolue
    /// (comme `bg_aliases` pour `background-image`).
    pub fn register_mask(&mut self, url: &str, img: Arc<DecodedImage>) {
        self.masks_url.insert(url.to_string(), img);
    }
}

/// Décode `bytes` en RGBA — `None` si le format n'est pas
/// reconnu ou si l'image dépasse `max_px` pixels (borne
/// mémoire : 4 octets/pixel → `max_px` plafonne l'allocation
/// *avant* décodage via les limites du décodeur).
/// SVG (étape 45) est détecté par le sniff et rasterisé via
/// `resvg`/`usvg` — statique : SMIL et scripts SVG ignorés.
#[must_use]
pub fn decode_image(bytes: &[u8], max_px: u32) -> Option<DecodedImage> {
    if looks_like_svg(bytes) {
        return decode_svg_raster(bytes, max_px);
    }
    let reader = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .ok()?;
    // Borne de décodage : côté carré max déduit du budget
    // pixels (une dimension peut excéder si l'autre est
    // minuscule — le produit reste borné à la vérification).
    let side = max_px;
    let mut limits = image::Limits::no_limits();
    limits.max_image_width = Some(side);
    limits.max_image_height = Some(side);
    let mut reader = reader;
    reader.limits(limits);
    let dyn_img = reader.decode().ok()?;
    let (w, h) = (dyn_img.width(), dyn_img.height());
    if w == 0 || h == 0 || u64::from(w) * u64::from(h) > u64::from(max_px) {
        return None;
    }
    let rgba = dyn_img.into_rgba8().into_vec();
    Some(DecodedImage {
        width: w,
        height: h,
        rgba,
    })
}

/// Sniff SVG (étape 45) : le document commence par `<` après
/// BOM/espaces et porte un élément `<svg` dans son prolog —
/// `usvg` valide ensuite strictement (un HTML avec `<svg>`
/// embarqué est refusé, root non-SVG).
fn looks_like_svg(bytes: &[u8]) -> bool {
    let head = &bytes[..bytes.len().min(4096)];
    let Ok(s) = core::str::from_utf8(head) else {
        return false;
    };
    let t = s.trim_start_matches('\u{feff}').trim_start();
    t.starts_with('<') && t.contains("<svg")
}

/// Rasterise un document SVG via `usvg`+`resvg` — taille
/// naturelle (`width`/`height`/`viewBox`), réduite
/// proportionnellement si elle dépasse `max_px`. `None` si
/// le document est refusé, sans géométrie ou trop grand.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
// les dims sont vérifiées finies, > 0 et ≤ `max_px` juste au-
// dessus — le cast ne peut ni tronquer ni perdre le signe.
/// Aussi utilisé par le `<svg>` inline (R-9, module `svg`).
pub(crate) fn decode_svg_raster(bytes: &[u8], max_px: u32) -> Option<DecodedImage> {
    // Parse confiné aussi — un document hostile peut faire
    // paniquer `usvg` (même politique §B-55 que `resvg::render`).
    let tree = std::panic::catch_unwind(|| {
        resvg::usvg::Tree::from_data(bytes, &resvg::usvg::Options::default())
    })
    .ok()?
    .ok()?;
    let size = tree.size();
    let (w, h) = (f64::from(size.width()), f64::from(size.height()));
    if !(w.is_finite() && h.is_finite()) || w <= 0.0 || h <= 0.0 {
        return None;
    }
    // Borne pixels : échelle de réduction si le naturel
    // dépasse le budget — un SVG « infini » (viewBox géant)
    // reste plafonné comme un bitmap hostile.
    let scale = if w * h > f64::from(max_px) {
        (f64::from(max_px) / (w * h)).sqrt()
    } else {
        1.0
    };
    let iw = u32::try_from((w * scale).ceil() as u64).ok()?.max(1);
    let ih = u32::try_from((h * scale).ceil() as u64).ok()?.max(1);
    if u64::from(iw) * u64::from(ih) > u64::from(max_px) {
        return None;
    }
    let mut pm = tiny_skia::Pixmap::new(iw, ih)?;
    let tf = tiny_skia::Transform::from_scale(scale as f32, scale as f32);
    // §B-55 : un SVG hostile (filtre `feComposite` à sources de
    // tailles divergentes) panique **dans** la dépendance resvg
    // (`filter/composite.rs` assert). Un asset ne doit jamais
    // emporter le raster — le panic est confiné et l'image
    // dégrade en `None` (repli alt/fond), comme tout illisible.
    let rendered = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        resvg::render(&tree, tf, &mut pm.as_mut());
    }));
    if rendered.is_err() {
        return None;
    }
    // tiny-skia stocke du prémultiplié — `DecodedImage`
    // contracte du RGBA droit : démultiplie pixel à pixel.
    let mut rgba = pm.data().to_vec();
    for px in rgba.chunks_exact_mut(4) {
        if let Some(c) = tiny_skia::PremultipliedColorU8::from_rgba(px[0], px[1], px[2], px[3]) {
            let d = c.demultiply();
            px[0] = d.red();
            px[1] = d.green();
            px[2] = d.blue();
            px[3] = d.alpha();
        }
    }
    Some(DecodedImage {
        width: iw,
        height: ih,
        rgba,
    })
}
