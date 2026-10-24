// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Attribution mémoire par réservoir (étape 130) — réponse au
//! constat terrain « station ~1,4 Gio sous sites réels ».
//!
//! [`Session::mem_report`] mesure un onglet vivant **et ses
//! pages gelées bfcache** — le premier réservoir suspecté :
//! chaque `PageSnapshot` gèle un moteur JS entier (heap borné
//! à `js_mem_limit_mib` chacun) + DOM + arbre + display list +
//! frame + PNG + images, `bfcache_pages` = 4 → cinq états de
//! page par onglet.
//!
//! Honnêteté de la mesure :
//!
//! - **Octets exacts** où NAV possède les tampons : `frame.
//!   pixels`, `png`, bitmaps `DecodedImage.rgba`, corps
//!   `bg_bodies` — et le heap JS via `JsEngine::heap_stats`.
//! - **Comptes** pour les structures domaine sans accesseur
//!   de capacité (`Dom.live_count`, boîtes de layout, items
//!   de display list) : le rapport distingue `_nodes`/`_boxes`
//!   (comptes) de `_bytes` (octets mesurés) — jamais un faux
//!   total octet inventé sur les structures PIPE.
//! - **Dédup `Arc`** : une même `DecodedImage` peut figurer
//!   dans `images.*`, les items de la display list et les
//!   caches `img_decoded`/`bg_decoded` — l'empreinte ne la
//!   compte qu'une fois par onglet.
//! - Non mesuré, documenté : les caches de fontes
//!   (`GlyphCache`/`FontCache` — pas d'accesseur), les styles
//!   calculés `prev_styles`, le DOM **miroir** interne de
//!   `explorer-jscore` (hors `heap_stats`) et les registres
//!   IndexedDB/caches (JS-C72 — `Rc` injectables hors heap).

use std::collections::HashMap as StdHashMap;

use explorer_render::DisplayItem;

use super::*;

/// Attribution mémoire d'un onglet — page vivante + pages
/// gelées bfcache. `_nodes`/`_boxes`/`_items` = comptes,
/// `_bytes` = octets mesurés (voir l'en-tête du module pour
/// la couverture honnête).
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct MemReport {
    /// Nœuds vivants du DOM courant (`Dom::live_count`).
    pub dom_nodes: usize,
    /// Boîtes de l'arbre de layout courant (descente récursive).
    pub layout_boxes: usize,
    /// Items de la display list courante.
    pub display_items: usize,
    /// `frame.pixels` de la frame courante (octets exacts).
    pub frame_bytes: usize,
    /// Taille du PNG servi (cache paresseux — `0` s'il n'a
    /// jamais été demandé).
    pub png_bytes: usize,
    /// Bitmaps décodées dé-dupliquées de la **page vivante**
    /// — `images.*` + display list + caches `img_decoded`/
    /// `bg_decoded` (octets `rgba` exacts). Le côté gelé est
    /// rapporté à part dans `bfcache_images_bytes` (étape
    /// 133 : attendu 0).
    pub images_bytes: usize,
    /// Corps `bg_bodies` pas encore décodés (octets exacts).
    pub bg_bodies_bytes: usize,
    /// Corps compressés retenus de la page vivante (étape
    /// 133 — source du re-décodage au retour bfcache).
    pub img_bodies_bytes: usize,
    /// Heap JS du contexte vivant (`heap_stats().estimated_bytes`).
    pub js_heap_bytes: usize,
    /// Nombre de pages gelées dans le bfcache.
    pub bfcache_pages: usize,
    /// Nœuds DOM des pages gelées (somme des `live_count`).
    pub bfcache_dom_nodes: usize,
    /// Boîtes de layout des pages gelées.
    pub bfcache_layout_boxes: usize,
    /// Items de display list des pages gelées.
    pub bfcache_display_items: usize,
    /// `frame.pixels` des pages gelées.
    pub bfcache_frame_bytes: usize,
    /// PNG cachés des pages gelées.
    pub bfcache_png_bytes: usize,
    /// Heaps JS gelés dans le bfcache — premier réservoir
    /// suspecté du constat 1,4 Gio (jusqu'à `bfcache_pages` ×
    /// `js_mem_limit_mib`).
    pub bfcache_js_heap_bytes: usize,
    /// Corps compressés retenus par les pages gelées (étape
    /// 133 — survivent au gel pour le re-décodage).
    pub bfcache_img_bodies_bytes: usize,
    /// Bitmaps encore référencées par les pages gelées —
    /// post-étape 133, attendu **0** : les `Arc` des
    /// `images.*`/`list`/`masks_url` sont lâchés au freeze ;
    /// non nul = référence résiduelle à investiguer.
    /// Sous-ensemble diagnostique (déjà couvert par
    /// `images_bytes` dé-dupliqué) — non compté dans
    /// `total_bytes`.
    pub bfcache_images_bytes: usize,
    /// Somme des champs `_bytes` de l'onglet (vivant + gelé)
    /// — hors structures à compte seul, honnête.
    pub total_bytes: usize,
}

/// Compte les boîtes d'un arbre de layout (descente récursive
/// — les `children` sont `pub`, contrat PIPE).
fn count_boxes(b: &LayoutBox) -> usize {
    1 + b.children.iter().map(count_boxes).sum::<usize>()
}

/// Ensemble de dédup bitmaps : `Arc::as_ptr` → `rgba.len()`
/// (la longueur est lue à l'insertion — jamais par
/// déréférencement du pointeur après coup).
type ImgSet = StdHashMap<*const DecodedImage, usize>;

/// Enregistre les bitmaps d'une `ImageTable` dans l'ensemble
/// de dédup — clé `Arc::as_ptr`, valeur `rgba.len()`.
fn collect_imgs<'a>(seen: &mut ImgSet, t: impl Iterator<Item = &'a std::sync::Arc<DecodedImage>>) {
    for img in t {
        seen.entry(std::sync::Arc::as_ptr(img))
            .or_insert(img.rgba.len());
    }
}

/// Somme des octets de bitmaps déjà référencées par une
/// `PaintImages` + ses items de display list + caches de la
/// page — toutes alimentent le même ensemble de dédup.
fn collect_page_imgs(seen: &mut ImgSet, images: &PaintImages, list: Option<&DisplayList>) {
    collect_imgs(seen, images.replaced.values());
    collect_imgs(seen, images.backgrounds.values());
    collect_imgs(seen, images.svgs.values());
    collect_imgs(seen, images.masks.values());
    collect_imgs(seen, images.masks_url.values());
    if let Some(l) = list {
        for item in &l.items {
            match item {
                DisplayItem::Image { img, .. } | DisplayItem::BackgroundImage { img, .. } => {
                    seen.entry(std::sync::Arc::as_ptr(img))
                        .or_insert(img.rgba.len());
                }
                DisplayItem::MaskedFill { mask, .. } => {
                    seen.entry(std::sync::Arc::as_ptr(mask))
                        .or_insert(mask.rgba.len());
                }
                _ => {}
            }
        }
    }
}

impl Session {
    /// Attribution mémoire de l'onglet `tab` (étape 130) —
    /// `None` si l'onglet n'existe pas. La mesure lit les
    /// stats du moteur JS vivant et gelé (`heap_stats` est
    /// `&mut` : le worker jscore répond sur son canal — appel
    /// synchrone borné comme tout `JsEngine`).
    pub fn mem_report(&mut self, tab: TabId) -> Option<MemReport> {
        let entry = self.tabs.get_mut(&tab)?;
        let mut r = MemReport::default();
        let mut seen: ImgSet = StdHashMap::new();

        // Page vivante.
        if let Some(d) = &entry.dom {
            r.dom_nodes = d.live_count();
        }
        if let Some(t) = &entry.tree {
            r.layout_boxes = count_boxes(t);
        }
        if let Some(l) = &entry.list {
            r.display_items = l.items.len();
        }
        if let Some(f) = &entry.frame {
            r.frame_bytes = f.pixels.len();
        }
        if let Some(p) = &entry.png {
            r.png_bytes = p.len();
        }
        r.bg_bodies_bytes = entry.bg_bodies.values().map(Vec::len).sum();
        r.img_bodies_bytes = entry.img_bodies.values().map(Vec::len).sum();
        if let Some(js) = entry.js.as_mut() {
            if let Some(s) = js.heap_stats() {
                r.js_heap_bytes = s.estimated_bytes;
            }
        }
        collect_page_imgs(&mut seen, &entry.images, entry.list.as_ref());
        collect_imgs(&mut seen, entry.img_decoded.values());
        collect_imgs(
            &mut seen,
            entry.bg_decoded.values().filter_map(Option::as_ref),
        );

        // Pages gelées bfcache — même attribution, sommée ;
        // les bitmaps gelées alimentent un ensemble propre
        // (étape 133 : le diagnostic « le gelé ne garde plus
        // aucun `Arc` » vaut par réservoir).
        let mut seen_snap: ImgSet = StdHashMap::new();
        r.bfcache_pages = entry.bfcache.len();
        for snap in &mut entry.bfcache {
            if let Some(d) = &snap.dom {
                r.bfcache_dom_nodes += d.live_count();
            }
            if let Some(t) = &snap.tree {
                r.bfcache_layout_boxes += count_boxes(t);
            }
            if let Some(l) = &snap.list {
                r.bfcache_display_items += l.items.len();
            }
            if let Some(f) = &snap.frame {
                r.bfcache_frame_bytes += f.pixels.len();
            }
            if let Some(p) = &snap.png {
                r.bfcache_png_bytes += p.len();
            }
            if let Some(js) = snap.js.as_mut() {
                if let Some(s) = js.heap_stats() {
                    r.bfcache_js_heap_bytes += s.estimated_bytes;
                }
            }
            r.bfcache_img_bodies_bytes += snap.img_bodies.values().map(Vec::len).sum::<usize>();
            collect_page_imgs(&mut seen_snap, &snap.images, snap.list.as_ref());
        }
        r.bfcache_images_bytes = seen_snap.values().sum();
        r.images_bytes = seen.values().sum();
        r.total_bytes = r.frame_bytes
            + r.png_bytes
            + r.images_bytes
            + r.bg_bodies_bytes
            + r.img_bodies_bytes
            + r.js_heap_bytes
            + r.bfcache_frame_bytes
            + r.bfcache_png_bytes
            + r.bfcache_js_heap_bytes
            + r.bfcache_img_bodies_bytes;
        Some(r)
    }
}
