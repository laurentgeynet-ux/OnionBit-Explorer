// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! `explorer-render` — display list → framebuffer RGBA → PNG
//! (`tiny-skia`, ADR-0001/0004).
//!
//! Étape 22 : [`build_display_list`] aplatit l'arbre de layout
//! en [`DisplayItem`] ordonnés (fonds, bordures, texte shapé,
//! clips `overflow`) — la seule entrée du raster. Une frame =
//! un état cohérent : jamais de rasterisation d'un arbre en
//! cours de mutation.

#![forbid(unsafe_code)]

pub mod display;
pub mod fonts;
pub mod image;
pub mod raster;
pub mod woff;

pub use display::{
    build_display_list, build_display_list_full, build_display_list_imaged,
    build_display_list_scrolled, BorderStyle, DisplayItem, DisplayList,
};
pub use fonts::{DirFontSource, WebFontSource};
pub use image::{decode_image, DecodedImage, ImageTable, PaintImages};
pub use raster::{rasterize, Frame};
pub use woff::decode_font;
