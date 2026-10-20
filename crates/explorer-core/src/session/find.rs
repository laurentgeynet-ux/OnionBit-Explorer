// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Find-in-page (étape 57bis) : recherche textuelle sur les
//! boîtes `Text` du boxtree, surlignage peint en overlay dans
//! la display list (même mécanisme que le popup `<select>`)
//! et scroll-vers-occurrence.
//!
//! Granularité v1 : le surlignage couvre la **boîte** entière
//! contenant la correspondance — les runs texte étant déjà
//! découpés par le layout en-ligne, le rectangle reste serré
//! dans les cas courants. Un surlignage au caractère près
//! demanderait une API de métriques d'`explorer-layout`
//! (demande PIPE dans `docs/interface-equipes.md`).

use super::*;

/// Recherche courante d'un onglet — occurrences déjà
/// géométrisées sur le boxtree posé.
pub(crate) struct FindState {
    /// Requête normalisée (minuscules) — la re-coloration
    /// d'une nouvelle passe repart du DOM si le layout a
    /// refondu entre-temps.
    pub(crate) query: String,
    /// Rectangles des occurrences, dans l'ordre du document.
    pub(crate) rects: Vec<Rect>,
    /// Occurrence courante (mise en valeur plus forte).
    pub(crate) current: usize,
}

/// Occurrences de `query` dans les boîtes `Text` de
/// l'arbre — comparaison insensible à la casse, sur le
/// texte tel qu'affiché. Ordre = parcours du boxtree
/// (ordre du document posé).
pub(crate) fn find_text_boxes(bx: &LayoutBox, needle: &str) -> Vec<Rect> {
    let mut out = Vec::new();
    if needle.is_empty() {
        return out;
    }
    find_into(bx, needle, &mut out);
    out
}

fn find_into(bx: &LayoutBox, needle: &str, out: &mut Vec<Rect>) {
    if let Some(t) = &bx.text {
        if t.to_lowercase().contains(needle) {
            out.push(bx.rect);
        }
    }
    for c in &bx.children {
        find_into(c, needle, out);
    }
}

impl Session {
    /// Nouvelle recherche : compte les occurrences, saute à
    /// la première, invalide la frame. Une requête vide efface
    /// l'état (fermeture de la barre).
    pub fn find_in_page(&mut self, tab: TabId, query: &str) -> usize {
        let q = query.to_lowercase();
        let Some(entry) = self.tabs.get_mut(&tab) else {
            return 0;
        };
        if q.is_empty() {
            entry.find = None;
            let _ = self.repaint(tab);
            self.emit_invalidated(tab);
            return 0;
        }
        let rects = entry
            .tree
            .as_ref()
            .map(|t| find_text_boxes(t, &q))
            .unwrap_or_default();
        let n = rects.len();
        entry.find = Some(FindState {
            query: q,
            rects,
            current: 0,
        });
        if n > 0 {
            self.find_scroll(tab);
        } else {
            let _ = self.repaint(tab);
            self.emit_invalidated(tab);
        }
        n
    }

    /// Occurrence suivante/précédente (`dir` ±1 — boucle sur
    /// les bornes) — renvoie `(index_courant, total)`.
    pub fn find_step(&mut self, tab: TabId, dir: i64) -> (usize, usize) {
        let Some(entry) = self.tabs.get_mut(&tab) else {
            return (0, 0);
        };
        let n = entry.find.as_ref().map_or(0, |f| f.rects.len());
        if n == 0 {
            return (0, 0);
        }
        let cur = {
            let Some(f) = entry.find.as_mut() else {
                return (0, 0);
            };
            f.current = match dir.cmp(&0) {
                std::cmp::Ordering::Greater => (f.current + 1) % n,
                std::cmp::Ordering::Less => (f.current + n - 1) % n,
                std::cmp::Ordering::Equal => f.current,
            };
            f.current
        };
        self.find_scroll(tab);
        (cur, n)
    }

    /// État courant pour l'UI — `(requête, index, total)` ou
    /// `None` si aucune recherche active.
    #[must_use]
    pub fn find_state(&self, tab: TabId) -> Option<(String, usize, usize)> {
        let f = self.tabs.get(&tab)?.find.as_ref()?;
        Some((f.query.clone(), f.current, f.rects.len()))
    }

    /// Scroll vers l'occurrence courante (tiers supérieur du
    /// viewport) + invalidation.
    fn find_scroll(&mut self, tab: TabId) {
        let Some(y) = self
            .tabs
            .get(&tab)
            .and_then(|e| e.find.as_ref())
            .and_then(|f| f.rects.get(f.current.min(f.rects.len().saturating_sub(1))))
            .map(|r| r.y)
        else {
            return;
        };
        let vh = self.viewport.1;
        let _ = self.scroll_to_y(tab, (y - vh / 3.0).max(0.0));
    }
}
