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
        let mut snap = PageSnapshot {
            hist_idx: self.hist_idx,
            url: self.url.take(),
            title: self.title.take(),
            scroll_y: self.scroll_y,
            doc_h: self.doc_h,
            dom: self.dom.take(),
            tree: self.tree.take(),
            tree_epoch: self.tree_epoch,
            list: self.list.take(),
            frame: self.frame.take(),
            png: self.png.take(),
            focused: self.focused.take(),
            focused_key: self.focused_key.take(),
            hovered: self.hovered.take(),
            popup: self.popup.take(),
            find: self.find.take(),
            js: self.js.take(),
            images: std::mem::take(&mut self.images),
            linked_sheets: std::mem::take(&mut self.linked_sheets),
            adopted_sheets: self.adopted_sheets.clone(),
            defined_names: self.defined_names.clone(),
            prev_styles: std::mem::take(&mut self.prev_styles),
            dirty: std::mem::take(&mut self.dirty),
            emitted: self.emitted,
            last_req: self.last_req.take(),
            frames: std::mem::take(&mut self.frames),
            frame_nodes: std::mem::take(&mut self.frame_nodes),
            referrer_policy: self.referrer_policy.take(),
            base_url: self.base_url.take(),
            base_target: self.base_target.take(),
            // `<meta refresh>` (CAP-143) : le gel conserve le
            // **délai restant** — la restauration recale
            // l'échéance, un séjour dans le bfcache ne
            // consomme pas le compte à rebours.
            refresh: self
                .refresh_at
                .take()
                .map(|(t, u)| (t.saturating_duration_since(std::time::Instant::now()), u)),
            csp: std::mem::take(&mut self.csp),
            // Les contextes JS des cadres gèlent **dans le
            // moteur** (`js` emporté au-dessus) — la table de
            // liaison suit pour garder le routage `ctx` à la
            // restauration (étape 134).
            frame_ctxs: std::mem::take(&mut self.frame_ctxs),
            img_bodies: std::mem::take(&mut self.img_bodies),
            bg_aliases: std::mem::take(&mut self.bg_aliases),
            mask_aliases: std::mem::take(&mut self.mask_aliases),
        };
        // Étape 133 — le gelé ne garde que les corps
        // compressés : les bitmaps `DecodedImage` sont
        // libérés ici (tables `images.*` + display list,
        // porteuses d'`Arc`), re-décodés depuis `img_bodies`
        // au retour. Les op-lists `<canvas>` (pas des
        // bitmaps) survivent.
        snap.images.replaced.clear();
        snap.images.backgrounds.clear();
        snap.images.svgs.clear();
        snap.images.masks.clear();
        snap.images.masks_url.clear();
        snap.list = None;
        self.set_scroll(0.0);
        self.doc_h = 0.0;
        // Issue #19 : la page gelée emporte son moteur — la
        // table partagée est vidée pour que le document
        // suivant mesure 0 (stubs) plutôt que des rects
        // d'un autre document, jusqu'au prochain commit.
        self.clear_geometry();
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
        self.tree_epoch = snap.tree_epoch;
        self.list = snap.list;
        self.frame = snap.frame;
        self.png = snap.png;
        self.focused = snap.focused;
        self.focused_key = snap.focused_key;
        self.hovered = snap.hovered;
        self.popup = snap.popup;
        self.find = snap.find;
        self.js = snap.js;
        self.images = snap.images;
        self.linked_sheets = snap.linked_sheets;
        self.adopted_sheets = snap.adopted_sheets;
        self.defined_names = snap.defined_names;
        self.prev_styles = snap.prev_styles;
        self.dirty = snap.dirty;
        self.emitted = snap.emitted;
        self.spec_paint = false; // snapshot auto-cohérent (56c)
                                 // Le moteur détaché de l'ancien contexte est mort —
                                 // son drapeau et les tampons qui lui appartenaient
                                 // (53f) ne doivent pas survivre à la restauration.
        self.js_detached = None;
        self.js_pending_net.clear();
        self.js_pending_crypto.clear();
        self.js_pending_dyn.clear();
        self.js_pending_mod.clear();
        self.js_pending_attrs.clear();
        self.pending_doc_lifecycle.clear();
        self.pending_anim_births.clear();
        self.sync_floor = None;
        self.last_req = snap.last_req;
        self.frames = snap.frames;
        self.frame_nodes = snap.frame_nodes;
        self.referrer_policy = snap.referrer_policy;
        self.base_url = snap.base_url;
        self.base_target = snap.base_target;
        self.refresh_at = snap
            .refresh
            .map(|(d, u)| (std::time::Instant::now() + d, u));
        self.csp = snap.csp;
        self.frame_ctxs = snap.frame_ctxs;
        // Étape 133 : les corps compressés reviennent vivants
        // (re-gelables) — le re-décodage des bitmaps est fait
        // par l'appelant via `Session::restore_images`.
        self.img_bodies = snap.img_bodies;
        self.bg_aliases = snap.bg_aliases;
        self.mask_aliases = snap.mask_aliases;
        self.transitions.clear();
        // Idem animations : la restauration repart sur les
        // déclarations — lecture réenregistrée au premier
        // re-style (étape 55e).
        self.animations.clear();
        self.nav_epoch += 1;
        self.pending_nav.clear();
        // La position restaurée a son entrée d'historique —
        // plus de fenêtre « en vol » (étape 84) ni de files
        // `pushState`/`replaceState` différés (CAP-151 —
        // elles appartenaient au document en vol).
        self.hist_pending = false;
        self.pending_push.clear();
        self.pending_replace = None;
        // Géométrie du document restauré (issue #19) —
        // même contrat que `put_paint` : la table reflète
        // l'état commis, pas celui de la page d'avant.
        self.rebuild_geometry();
    }

    /// Base de résolution des URLs relatives (étape 135) —
    /// `<base href>` du document, ou son URL ; `None` = pas
    /// de document chargé.
    pub(crate) fn nav_base(&self) -> Option<&Url> {
        self.base_url.as_ref().or(self.url.as_ref())
    }

    /// Scroll commis — propage à la table de géométrie
    /// publiée (issue #19) **sans rebuild** : la conversion
    /// document→viewport se fait à la lecture. Les mutations
    /// directes de `scroll_y` hors navigation passent par
    /// ici (input scroll, restore de position).
    pub(crate) fn set_scroll(&mut self, y: f32) {
        self.scroll_y = y;
        if let Ok(mut g) = self.geom.lock() {
            g.scroll_y = y;
        }
    }

    /// Vide la table de géométrie — commit de navigation :
    /// les `element_path` du document mort seraient recyclés
    /// par le nouveau et serviraient de **fausses** mesures
    /// jusqu'au premier paint (issue #19 — un absent doit
    /// répondre 0, jamais l'état d'un autre document).
    pub(crate) fn clear_geometry(&mut self) {
        if let Ok(mut g) = self.geom.lock() {
            g.rects.clear();
            g.doc_w = 0.0;
            g.doc_h = 0.0;
        }
    }

    /// Re-remplit la table de géométrie publiée depuis
    /// l'état **commis** (`dom`/`tree`/`scroll_y`/`doc_h`) —
    /// issue #19 : appelée à chaque `put_paint` et à la
    /// restauration bfcache, les deux points où DOM + arbre
    /// reposent ensemble. Sans DOM/arbre (page vierge,
    /// fenêtre détachée) la table est vidée : mesurer un
    /// instantané absent doit répondre 0, pas un état
    /// périmé d'un autre document (les rects sont clés par
    /// `element_path` — des chemins recyclés serviraient de
    /// fausses mesures).
    pub(crate) fn rebuild_geometry(&mut self) {
        let Ok(mut g) = self.geom.lock() else {
            return;
        };
        g.rects.clear();
        g.scroll_x = 0.0;
        g.scroll_y = self.scroll_y;
        g.doc_w = 0.0;
        g.doc_h = self.doc_h;
        if let (Some(dom), Some(tree)) = (self.dom.as_ref(), self.tree.as_ref()) {
            g.doc_w = tree.rect.width;
            dom_helpers::collect_geometry(dom, tree, false, &mut g);
        }
    }
}
