// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Pipeline de peinture : cascade, layout, repaint, synchro
//! DOM incrémentale (R-12).
use super::*;
use std::collections::HashMap;
use std::time::Instant;

use explorer_css::{ComputedStyle, Origin, StyleEngine, StyleEnv, StyleMap};
use explorer_dom::Dom;
use explorer_html::TreeBuilder;
use explorer_layout::font::FontSource;
use explorer_layout::{
    build_box_tree_sized, layout_tree, relayout_subtree, BlockCtx, Rect, Relayout,
};
use explorer_net::url::Url;
use explorer_render::canvas::CanvasTable;
use explorer_render::{build_display_list_full, collect_data_images, rasterize, PaintImages};

impl Session {
    /// Resynchronise le document depuis le miroir JS muté :
    /// re-parse du HTML sérialisé, relecture du titre
    /// (`TitleChanged` si changé). `focused`/`hovered` sont
    /// conservés — ce sont des chemins, pas des `NodeId`
    /// (étape 52). `relayout` contrôle
    /// l'invalidation immédiate — vrai depuis les événements,
    /// faux pendant la navigation (qui relayout juste après).
    /// `dirty_path` = chemin du sous-arbre sale rapporté par
    /// le miroir (étape 55f) — sert à viser
    /// [`Self::try_sync_incremental`] au lieu du reflow global.
    pub(crate) fn apply_dom_sync(
        &mut self,
        tab: TabId,
        html: &str,
        dirty_path: Option<&[(String, u32)]>,
        relayout: bool,
    ) {
        let t0 = Instant::now();
        let new_dom = TreeBuilder::parse(html);
        let mut t = sync_phase(tab, "sync.reparse", t0);
        let new_title = extract_title(&new_dom);
        // Relayout incrémental (étape 55f) avant tout remplacement
        // — il a besoin de l'ancien DOM et de l'ancien arbre.
        let incremental =
            relayout && dirty_path.is_some_and(|p| self.try_sync_incremental(tab, &new_dom, p));
        if relayout {
            t = sync_phase(tab, "sync.incremental", t);
        }
        // Les op-lists `<canvas>` survivent au re-parse : le
        // miroir les conserve (bitmap cumulative) — la table
        // est construite sur `new_dom` **avant** le move dans
        // l'onglet (chemins → `NodeId` du DOM commis, JS-C28).
        let canvases = self
            .tabs
            .get_mut(&tab)
            .map_or_else(CanvasTable::new, |e| canvas_table(e, &new_dom));
        let _ = sync_phase(tab, "sync.canvas_table", t);
        if let Some(entry) = self.tabs.get_mut(&tab) {
            entry.dom = Some(new_dom);
            // `focused`/`hovered` sont des **chemins** — ils
            // survivent au re-parse et se re-résolvent à
            // l'usage (étape 52) ; un nœud supprimé
            // simplement ne résout plus.
            // Les `NodeId` changent au re-parse → les pixels
            // indexés par nœud sont obsolètes (les `<img>`
            // nouveaux retombent sur `alt` jusqu'à la
            // prochaine navigation — borne documentée).
            // En chemin incrémental `svgs`/`tree` sont déjà
            // à jour ; `replaced` seule est vidée.
            entry.images = PaintImages::default();
            entry.images.canvases = canvases;
            // Un re-parse de synchro rend obsolètes les
            // options figées du popup `<select>` — il se
            // referme (étape 54).
            entry.popup = None;
            if entry.title != new_title {
                entry.title.clone_from(&new_title);
                if let Some(t) = new_title {
                    let _ = self.events.send(CoreEvent::TitleChanged { tab, title: t });
                }
            }
        }
        if relayout {
            if !incremental {
                let tf = Instant::now();
                if let Err(e) = self.relayout(tab) {
                    tracing::debug!(tab, error = %e, "relayout après mutation js");
                    return;
                }
                sync_phase(tab, "sync.relayout_full", tf);
            }
            // Coalescence pompe (55g-suite) : la fenêtre avant
            // le prochain `sync_dom` de timer est
            // proportionnelle au coût de cet apply — bornée
            // [`paint_interval`, `resync_gap_max`].
            let floor = t0.elapsed().clamp(self.paint_interval, self.resync_gap_max);
            if let Some(entry) = self.tabs.get_mut(&tab) {
                entry.sync_floor = Some(Instant::now() + floor);
            }
            self.emit_invalidated(tab);
        }
    }

    /// Relayout incrémental d'une synchro DOM (étape 55f —
    /// adoption R-12) : le sous-arbre sale rapporté par le
    /// miroir est reboîté + re-posé en place par
    /// [`relayout_subtree`] **si** la cible résout au même
    /// `NodeId` dans l'ancien et le nouveau DOM (le préfixe
    /// structurel est alors stable — une mutation interne au
    /// sous-arbre ne décale pas son propre id). Les boîtes
    /// conservées portent d'anciens ids : re-mappés par
    /// chemin d'élément — un id qui ne se re-résout pas
    /// proprement renvoie `false` (l'appelant refait un
    /// reflow global : l'arbre mixte est alors entièrement
    /// reconstruit — toujours sûr).
    pub(crate) fn try_sync_incremental(
        &mut self,
        tab: TabId,
        new_dom: &Dom,
        path: &[(String, u32)],
    ) -> bool {
        // Phase 1 — résolution de la cible sous emprunt court.
        let resolve = |s: &Session, dom: &Dom| {
            let entry = s.tabs.get(&tab)?;
            let old_dom = entry.dom.as_ref()?;
            entry.tree.as_ref()?;
            let sizes = image_sizes(dom, &entry.images.replaced);
            Some((dom_path_node(old_dom, path)?, sizes))
        };
        let Some((target_old, mut sizes)) = resolve(self, new_dom) else {
            return false;
        };
        let Some(target_new) = dom_path_node(new_dom, path) else {
            return false;
        };
        let Some(root_el) = new_dom.document_element() else {
            return false;
        };
        let t0 = Instant::now();
        let styles = self.compute_styles(tab, new_dom);
        let t = sync_phase(tab, "incr.compute_styles", t0);
        sizes.extend(explorer_render::svg::svg_sizes(new_dom, root_el));
        let svgs = explorer_render::svg::collect_svg(new_dom, root_el, self.image_max_px);
        let ctx = BlockCtx {
            fonts: Some(&self.webfonts as &dyn FontSource),
            ..BlockCtx::default()
        };
        // Phase 2 — remap des ids conservés + relayout local,
        // sous emprunt mutable court. Le remap précède le
        // relayout : `bx.node == target` doit comparer des
        // ids du **nouveau** DOM (les indices d'arène ne sont
        // pas stables au re-parse — la sérialisation n'est
        // pas structure-identique).
        {
            let Some(entry) = self.tabs.get_mut(&tab) else {
                return false;
            };
            let (Some(old_dom), Some(tree)) = (entry.dom.as_ref(), entry.tree.as_mut()) else {
                return false;
            };
            if !remap_box_nodes(old_dom, new_dom, tree, target_old, target_new) {
                return false; // id irrésolvable → reflow global de l'appelant
            }
            if relayout_subtree(tree, new_dom, &styles, &sizes, target_new, &ctx) != Relayout::Done
            {
                return false; // arbre mixte jeté → fallback sûr
            }
            entry.doc_h = tree.rect.height;
            entry.images.svgs = svgs;
            // `<canvas>` (JS-C28) — les boîtes re-mappées
            // portent les `NodeId` du **nouveau** DOM.
            entry.images.canvases = canvas_table(entry, new_dom);
        }
        let tr = sync_phase(tab, "incr.remap_relayout", t);
        if self.repaint(tab).is_ok() {
            sync_phase(tab, "incr.repaint", tr);
            self.incremental_syncs += 1;
            return true;
        }
        false
    }

    /// `Navigated` + `TitleChanged` (si le titre change) +
    /// `FrameInvalidated` — séquence d'événements post-rendu.
    /// `prev` = titre avant navigation (capturé par l'appelant,
    /// `navigate_to` l'ayant déjà écrasé).
    pub(crate) fn emit_navigated(
        &mut self,
        tab: TabId,
        url: &Url,
        title: Option<String>,
        prev: Option<&str>,
    ) {
        let _ = self.events.send(CoreEvent::Navigated {
            tab,
            url: url.to_string(),
            title: title.clone(),
        });
        if let Some(t) = title.filter(|t| prev != Some(t.as_str())) {
            let _ = self.events.send(CoreEvent::TitleChanged { tab, title: t });
        }
        self.emit_invalidated(tab);
    }

    /// Log + `PipelineError` sur le bus, renvoie l'erreur.
    pub(crate) fn emit_error(&mut self, tab: TabId, e: CoreError) -> CoreError {
        tracing::debug!(tab, error = %e, "pipeline en échec");
        let _ = self.events.send(CoreEvent::PipelineError {
            tab: Some(tab),
            message: e.to_string(),
        });
        e
    }

    /// Re-pose l'arbre et re-peint depuis l'état courant
    /// (resize, style muté) — sans re-fetch.
    ///
    /// # Errors
    /// [`CoreError::UnknownTab`] ou [`CoreError::EmptyDocument`]
    /// si l'onglet n'a jamais été navigué.
    pub fn rerender(&mut self, tab: TabId) -> Result<(), CoreError> {
        if !self.tabs.contains_key(&tab) {
            return Err(CoreError::UnknownTab(tab));
        }
        self.relayout(tab)?;
        self.emit_invalidated(tab);
        Ok(())
    }

    /// Style → boîtes → layout → display list → raster → PNG.
    pub(crate) fn relayout(&mut self, tab: TabId) -> Result<(), CoreError> {
        // Le DOM sort de l'onglet le temps du pipeline —
        // `relayout_dom` veut `&mut self` et un `&Dom` disjoint.
        let dom = self
            .tabs
            .get_mut(&tab)
            .and_then(|e| e.dom.take())
            .ok_or(CoreError::EmptyDocument)?;
        let r = self.relayout_dom(tab, &dom);
        if let Some(e) = self.tabs.get_mut(&tab) {
            e.dom = Some(dom);
        }
        r
    }

    /// Pipeline style → layout → raster sur `dom` (emprunté —
    /// `relayout` pour le DOM posé, `nav_paint` pour le DOM
    /// en construction). Écrit `tree`/`doc_h`/frame/png.
    /// Passe de styles partagée par le reflow complet et le
    /// relayout incrémental (étape 55f) : cascade `<style>` +
    /// `<link>`, calcul sur tout `dom`, surcharge des
    /// transitions en vol. Effet de bord : `prev_styles`/
    /// `transitions` de l'onglet mis à jour.
    pub(crate) fn compute_styles(&mut self, tab: TabId, dom: &Dom) -> StyleMap {
        // Feuilles : `<style>` inline puis `<link>` fetchées
        // (étape 46) — même origine auteur, ordre source.
        let linked = self
            .tabs
            .get(&tab)
            .map_or_else(String::new, |e| e.linked_css.clone());
        let mut eng = StyleEngine::new();
        eng.add_css(&collect_style_css(dom), Origin::Author);
        eng.add_css(&linked, Origin::Author);
        let mut styles = eng.compute_tree(dom, dom.document(), &StyleEnv::default());
        // Transitions (étape 45) : diff sur le brut, surcharge
        // des progressions — `prev_styles` conserve le map
        // AVANT animation, indexé par ordinal d'élément (le
        // `NodeId` meurt au prochain re-parse).
        let ord_of = element_ordinals(dom);
        let raw: HashMap<usize, ComputedStyle> = styles
            .iter()
            .filter_map(|(n, c)| ord_of.get(&n).map(|o| (*o, c.clone())))
            .collect();
        if let Some(e) = self.tabs.get_mut(&tab) {
            let now = Instant::now();
            update_transitions(&e.prev_styles, &mut e.transitions, &styles, &ord_of, now);
            // `@keyframes` (étape 55e) : identité
            // (élément, index) + déclaration → `start`
            // conservé, puis surcharges — keyframes sous
            // transitions (couche spec).
            sync_animations(&mut e.animations, &styles, &ord_of, &eng, now);
            apply_anim_overrides(
                &e.transitions,
                &mut e.animations,
                &eng,
                &mut styles,
                &ord_of,
                now,
            );
            e.prev_styles = raw;
        }
        styles
    }

    pub(crate) fn relayout_dom(&mut self, tab: TabId, dom: &Dom) -> Result<(), CoreError> {
        let styles = self.compute_styles(tab, dom);
        let entry = self.tabs.get_mut(&tab).ok_or(CoreError::UnknownTab(tab))?;
        let Some(root_el) = dom.document_element() else {
            return Err(CoreError::EmptyDocument);
        };
        // Images `data:` inline (étape 56f) : zéro réseau —
        // `<img src=data:>`/`<object data=data:>` → `replaced`
        // (sert le sizing intrinsèque juste après),
        // `background-image: url(data:)` en cascade →
        // `backgrounds`. Fusionnées avec les corps fetchés.
        let data_imgs = collect_data_images(dom, &styles, self.image_max_px);
        entry.images.replaced.extend(data_imgs.replaced);
        entry.images.backgrounds.extend(data_imgs.backgrounds);
        let mut sizes = image_sizes(dom, &entry.images.replaced);
        // `<svg>` inline (étape 55b) : tailles intrinsèques
        // `width`/`height`/`viewBox` pour la mise en boîte,
        // bitmap rasterisé une fois par reflow pour la
        // peinture (borné `image_max_px`, sous-arbres hostiles
        // refusés → la boîte retombe remplacée sans image).
        sizes.extend(explorer_render::svg::svg_sizes(dom, root_el));
        let svgs = explorer_render::svg::collect_svg(dom, root_el, self.image_max_px);
        let Some(mut bx) = build_box_tree_sized(dom, root_el, &styles, &sizes) else {
            return Err(CoreError::NothingToPaint);
        };
        let (vw, _vh) = self.viewport;
        let ctx = BlockCtx {
            fonts: Some(&self.webfonts as &dyn FontSource),
            ..BlockCtx::default()
        };
        layout_tree(&mut bx, vw, &ctx);
        entry.doc_h = bx.rect.height;
        entry.tree = Some(bx);
        entry.images.svgs = svgs;
        // `<canvas>` 2D (JS-C28) : op-lists cumulées du miroir,
        // re-résolues par chemin d'élément dans `dom` (les
        // `NodeId` commis indexent la table).
        entry.images.canvases = canvas_table(entry, dom);
        self.repaint(tab)
    }

    /// Display list + raster + PNG depuis l'arbre posé —
    /// sans re-layout (scroll, invalidation partielle).
    pub(crate) fn repaint(&mut self, tab: TabId) -> Result<(), CoreError> {
        let entry = self.tabs.get(&tab).ok_or(CoreError::UnknownTab(tab))?;
        let Some(bx) = entry.tree.as_ref() else {
            return Err(CoreError::EmptyDocument);
        };
        let (vw, vh) = self.viewport;
        let mut list = build_display_list_full(bx, vw, vh, entry.scroll_y, &entry.images);
        // Popup `<select>` ouvert : overlay chrome peint en
        // dernier — au-dessus de tout contenu (étape 54).
        if let Some(pop) = &entry.popup {
            append_select_popup(&mut list, pop, vw, entry.doc_h);
        }
        // Document géant : plafonner la hauteur rasterisée
        // plutôt qu'échouer — le PNG couvre le sommet du
        // document (dégradation, cf. `MAX_FRAME_DIM`). La
        // borne du raster mesure des pixels : le cap CSS
        // se divise par `scale`.
        let frame_cap = MAX_FRAME_DIM / self.scale.max(f32::EPSILON);
        if list.height > frame_cap {
            list.height = frame_cap;
        }
        // Hors-champ raster (étape 55g) : jeter les items
        // entièrement sous la borne de hauteur ou hors cadre
        // — `paint_text` re-dessinait chaque glyphe du
        // document, visible ou non (~18 s sur ~3 000 nœuds).
        cull_display_list(&mut list);
        let frame = rasterize(&list, &self.webfonts as &dyn FontSource, self.scale)
            .ok_or(CoreError::FrameTooBig)?;
        let entry = self.tabs.get_mut(&tab).ok_or(CoreError::UnknownTab(tab))?;
        // Dirty-rects (étape 55) : diff avec la liste
        // précédente — première peinture ou listes
        // incomparables → toute la frame (conservateur).
        let regions = entry.list.as_ref().map_or_else(
            || {
                vec![Rect {
                    x: 0.0,
                    y: 0.0,
                    width: list.width,
                    height: list.height,
                }]
            },
            |prev| {
                // Au-delà du seuil, la fusion O(k²) des rects
                // de `changed_rects` coûte plus cher que la
                // pleine frame qu'elle produirait de toute
                // façon (étape 55g — ~2 s mesurés pour
                // ~8 500 rects sur ~3 000 nœuds).
                if list.items.len() > DIRTY_DIFF_MAX_ITEMS
                    || prev.items.len() > DIRTY_DIFF_MAX_ITEMS
                {
                    vec![Rect {
                        x: 0.0,
                        y: 0.0,
                        width: list.width,
                        height: list.height,
                    }]
                } else {
                    list.changed_rects(prev)
                }
            },
        );
        entry.dirty.extend(regions);
        if entry.dirty.len() > MAX_DIRTY_RECTS {
            // Collapse « toute la frame » — le watermark est
            // ré-armé : le plein cadre sera rapporté à la
            // prochaine émission.
            entry.emitted = 0;
            entry.dirty = vec![Rect {
                x: 0.0,
                y: 0.0,
                width: list.width,
                height: list.height,
            }];
        }
        entry.list = Some(list);
        entry.frame = Some(frame);
        // Invalidation du cache PNG (étape 55g) — l'encodage
        // est paresseux, servi par `frame_png` à la demande.
        entry.png = None;
        // Une peinture aboutie produit un état cohérent
        // (étape 56c) — le drapeau spéculatif est ré-armé
        // par `nav_paint` **après** son propre appel ici.
        entry.spec_paint = false;
        Ok(())
    }
}
