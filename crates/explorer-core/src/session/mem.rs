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
//!   `bg_bodies`/`img_bodies`, corps POST de l'historique,
//!   textes CSS `linked_sheets`/`adopted_sheets` — et le
//!   heap JS via `JsEngine::heap_stats`.
//! - **Tampons de livrées JS** (`js_pending_*`) : corps
//!   réseau, scripts dynamiques/modules, résultats
//!   `crypto.subtle`, charges `EventSource`/`WebSocket`
//!   accumulées pendant un détachement moteur — un burst
//!   fetch/XHR non drainé y vit en octets exacts (étape
//!   168 : premier suspect octet-identifié du bras
//!   §B-59 après le bornage `flow_columns` R-136).
//! - **Comptes** pour les structures domaine sans accesseur
//!   de capacité (`Dom.live_count`, boîtes de layout, items
//!   de display list) : le rapport distingue `_nodes`/`_boxes`
//!   (comptes) de `_bytes` (octets mesurés) — jamais un faux
//!   total octet inventé sur les structures PIPE.
//! - **Dédup `Arc`** : une même `DecodedImage` peut figurer
//!   dans `images.*`, les items de la display list, les
//!   surfaces `frame_bitmaps` et les caches `img_decoded`/
//!   `bg_decoded` — l'empreinte ne la compte qu'une fois
//!   par onglet.
//! - Non mesuré, documenté : les caches de fontes
//!   (`GlyphCache`/`FontCache` — pas d'accesseur), les styles
//!   calculés `prev_styles` (compte seul), le DOM **miroir**
//!   interne de `explorer-jscore` (hors `heap_stats`), les
//!   buffers MSE des surfaces média claimées par l'hôte et
//!   les registres IndexedDB/caches (JS-C72 — `Rc`
//!   injectables hors heap).

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
    /// Corps réseau tamponnés pendant un détachement JS
    /// (`js_pending_net` — livrés à la réinsertion ; un
    /// burst fetch/XHR non drainé gonfle ce réservoir —
    /// bras §B-59 instrumenté étape 168).
    pub pending_net_bytes: usize,
    /// Corps des `<script>` dynamiques et sources de
    /// graphes de modules tamponnés pendant le même
    /// détachement (`js_pending_dyn`/`js_pending_mod`).
    pub pending_dyn_bytes: usize,
    /// Charges des livrées auxiliaires en attente :
    /// résultats `crypto.subtle`, `data` `EventSource`,
    /// messages `WebSocket` (texte/binaire).
    pub pending_aux_bytes: usize,
    /// Livrées en attente tous tampons confondus (compte :
    /// `js_pending_*`, cycle de vie document, naissances
    /// d'animations, pushes d'attributs, intentions de
    /// navigation, inputs différés).
    pub pending_events: usize,
    /// Textes CSS `linked_sheets` + `adopted_sheets` de la
    /// page vivante (octets exacts).
    pub sheets_bytes: usize,
    /// Corps POST conservés dans l'historique local —
    /// l'entrée stocke la **requête** entière pour la
    /// rejouer à la traversée (étape 56).
    pub hist_body_bytes: usize,
    /// Entrées de l'historique local (compte).
    pub hist_entries: usize,
    /// Styles calculés de la dernière mise en page
    /// (`prev_styles`, indexés par ordinal d'élément —
    /// compte seulement : `ComputedStyle` n'est pas
    /// mesurable à l'octet sans instrumentation PIPE).
    pub prev_styles: usize,
    /// Surfaces média offertes de la page vivante (compte —
    /// les buffers MSE vivent dans le moteur ou l'hôte
    /// claimé, hors ce rapport).
    pub media_surfaces: usize,
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
    /// Journaux rejoués en place (session-globale, pas par
    /// onglet) — taux de succès de la voie différentielle
    /// ADR-0025 (étape 137c — calibrage du seuil de repli).
    pub journal_applies: u32,
    /// Replis re-parse après journal tenté (session-globale).
    pub journal_fallbacks: u32,
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
    #[allow(clippy::too_many_lines)] // attribution par réservoir : somme plate, pas de sous-fonction nette
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
        // Tampons de livrées JS — octets exacts, premier
        // suspect du bras §B-59 post-R-136 (un détachement
        // long + burst réseau = octets retenus hors heap).
        r.pending_net_bytes = entry.js_pending_net.iter().map(|d| d.body.len()).sum();
        r.pending_dyn_bytes = entry
            .js_pending_dyn
            .iter()
            .filter_map(|d| d.2.as_ref())
            .map(String::len)
            .sum::<usize>()
            + entry
                .js_pending_mod
                .iter()
                .filter_map(|d| d.2.as_ref())
                .map(|m| m.graph.sources.values().map(String::len).sum::<usize>())
                .sum::<usize>();
        r.pending_aux_bytes = entry
            .js_pending_crypto
            .iter()
            .filter_map(|c| c.result.as_ref().ok())
            .map(Vec::len)
            .sum::<usize>()
            + entry
                .js_pending_es
                .iter()
                .map(|(_, _, ev)| match ev {
                    explorer_js::EsEvent::Message { data, event, id } => {
                        data.len() + event.as_ref().map_or(0, String::len) + id.len()
                    }
                    _ => 0,
                })
                .sum::<usize>()
            + entry
                .js_pending_ws
                .iter()
                .map(|(_, _, ev)| match ev {
                    explorer_js::WsEventKind::Text(t) => t.len(),
                    explorer_js::WsEventKind::Binary(b) => b.len(),
                    explorer_js::WsEventKind::Open(s) => s.len(),
                    explorer_js::WsEventKind::Close { reason, .. } => reason.len(),
                    explorer_js::WsEventKind::Error => 0,
                })
                .sum::<usize>();
        r.pending_events = entry.js_pending_net.len()
            + entry.js_pending_dyn.len()
            + entry.js_pending_mod.len()
            + entry.js_pending_crypto.len()
            + entry.js_pending_es.len()
            + entry.js_pending_ws.len()
            + entry.js_pending_attrs.len()
            + entry.pending_doc_lifecycle.len()
            + entry.pending_anim_births.len()
            + entry.pending_nav.len()
            + entry.pending_nav_reqs.len()
            + entry.pending_push.len()
            + usize::from(entry.pending_replace.is_some())
            + entry.deferred_inputs.len();
        r.sheets_bytes = entry
            .linked_sheets
            .iter()
            .chain(entry.adopted_sheets.iter())
            .map(String::len)
            .sum();
        r.hist_entries = entry.hist.len();
        r.hist_body_bytes = entry
            .hist
            .iter()
            .filter_map(|e| e.req.body.as_ref())
            .map(Vec::len)
            .sum();
        r.prev_styles = entry.prev_styles.len();
        r.media_surfaces = entry.media.len();
        collect_imgs(&mut seen, entry.frame_bitmaps.values());
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
            + r.pending_net_bytes
            + r.pending_dyn_bytes
            + r.pending_aux_bytes
            + r.sheets_bytes
            + r.hist_body_bytes
            + r.js_heap_bytes
            + r.bfcache_frame_bytes
            + r.bfcache_png_bytes
            + r.bfcache_js_heap_bytes
            + r.bfcache_img_bodies_bytes;
        // 137c — taux de succès de la voie différentielle,
        // session-globale (seuil de repli à calibrer en vols).
        r.journal_applies = self.journal_applies;
        r.journal_fallbacks = self.journal_fallbacks;
        Some(r)
    }
}
