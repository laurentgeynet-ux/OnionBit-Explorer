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

/// Décode `bytes` en RGBA — `None` si le format n'est pas
/// reconnu ou si l'image dépasse `max_px` pixels (borne
/// mémoire : 4 octets/pixel → `max_px` plafonne l'allocation
/// *avant* décodage via les limites du décodeur).
#[must_use]
pub fn decode_image(bytes: &[u8], max_px: u32) -> Option<DecodedImage> {
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
