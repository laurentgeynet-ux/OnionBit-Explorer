// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Méthodes internes de `Tab` — l'état d'onglet vit dans
//! `session/mod.rs` (champs privés visibles du sous-arbre).
use super::*;

impl Tab {
    /// Gèle la page courante dans le bfcache à sa position
    /// `hist_idx` (étape 56) — appelé à chaque navigation
    /// confirmée ([`Session::nav_apply`]). Sans page réelle
    /// (onglet vierge) → no-op. Borne `cap` : FIFO.
    pub(crate) fn freeze(&mut self, cap: usize) {
        // Rien à geler tant que la page n'a pas de produit
        // peint — un DOM commis sans frame (fenêtre
        // `nav_apply` → `sub_commit`, étape 56b) produirait
        // un snapshot empoisonné : sa restauration servirait
        // un `/frame` 204 persistant avec hit-test mort.
        // Mieux vaut re-fetcher la position plus tard.
        if self.dom.is_none() || self.frame.is_none() {
            return;
        }
        // Une page ne vit qu'une fois par position —
        // une nouvelle visite remplace l'ancienne gelée.
        self.bfcache.retain(|s| s.hist_idx != self.hist_idx);
        let snap = PageSnapshot {
            hist_idx: self.hist_idx,
            url: self.url.take(),
            title: self.title.take(),
            scroll_y: self.scroll_y,
            doc_h: self.doc_h,
            dom: self.dom.take(),
            tree: self.tree.take(),
            list: self.list.take(),
            frame: self.frame.take(),
            png: self.png.take(),
            focused: self.focused.take(),
            hovered: self.hovered.take(),
            popup: self.popup.take(),
            find: self.find.take(),
            js: self.js.take(),
            images: std::mem::take(&mut self.images),
            linked_css: std::mem::take(&mut self.linked_css),
            prev_styles: std::mem::take(&mut self.prev_styles),
            dirty: std::mem::take(&mut self.dirty),
            emitted: self.emitted,
            last_req: self.last_req.take(),
        };
        self.scroll_y = 0.0;
        self.doc_h = 0.0;
        // `dirty` vient d'être gelée → le live repart à
        // vide, watermark ré-armé.
        self.emitted = 0;
        self.bfcache.push_back(snap);
        while self.bfcache.len() > cap {
            self.bfcache.pop_front();
        }
    }

    /// Retire le snapshot gelé à `idx` s'il existe (étape
    /// 56 — le hit bfcache d'un `history_move`).
    pub(crate) fn take_snapshot(&mut self, idx: usize) -> Option<PageSnapshot> {
        let pos = self.bfcache.iter().position(|s| s.hist_idx == idx)?;
        self.bfcache.remove(pos)
    }

    /// Restaure un snapshot : tout l'état de page revient
    /// tel quel — epoch incrémentée (invalide toute nav en
    /// vol) et intentions de navigation purgées (un script
    /// de la page gelée ne doit pas tirer des navs périmées).
    pub(crate) fn restore(&mut self, snap: PageSnapshot) {
        self.url = snap.url;
        self.title = snap.title;
        self.scroll_y = snap.scroll_y;
        self.doc_h = snap.doc_h;
        self.dom = snap.dom;
        self.tree = snap.tree;
        self.list = snap.list;
        self.frame = snap.frame;
        self.png = snap.png;
        self.focused = snap.focused;
        self.hovered = snap.hovered;
        self.popup = snap.popup;
        self.find = snap.find;
        self.js = snap.js;
        self.images = snap.images;
        self.linked_css = snap.linked_css;
        self.prev_styles = snap.prev_styles;
        self.dirty = snap.dirty;
        self.emitted = snap.emitted;
        self.spec_paint = false; // snapshot auto-cohérent (56c)
                                 // Le moteur détaché de l'ancien contexte est mort —
                                 // son drapeau et les tampons qui lui appartenaient
                                 // (53f) ne doivent pas survivre à la restauration.
        self.js_detached = None;
        self.js_pending_net.clear();
        self.js_pending_attrs.clear();
        self.sync_floor = None;
        self.last_req = snap.last_req;
        self.transitions.clear();
        // Idem animations : la restauration repart sur les
        // déclarations — lecture réenregistrée au premier
        // re-style (étape 55e).
        self.animations.clear();
        self.nav_epoch += 1;
        self.pending_nav.clear();
    }
}
