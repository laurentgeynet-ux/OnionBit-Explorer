// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Find-in-page (étape 57bis) : recherche textuelle sur les
//! boîtes `Text` du boxtree, surlignage peint en overlay dans
//! la display list (même mécanisme que le popup `<select>`)
//! et scroll-vers-occurrence.
//!
//! Granularité (R-35) : la correspondance est localisée au
//! **glyphe** près — `fragment_sub_rect` (PIPE) donne le
//! sous-rect de la tranche d'octets couverte dans chaque
//! fragment posé. La recherche porte sur le texte tel
//! qu'**affiché** (`frag.text` — espaces collapsés inclus) ;
//! une occurrence qui traverse plusieurs fragments — voire
//! un retour à la ligne — groupe leurs sous-rects. Les boîtes
//! `Text` sans couverture de fragments (valeurs de contrôle)
//! gardent le surlignage boîte-entière en repli.

use super::*;

/// Recherche courante d'un onglet — occurrences déjà
/// géométrisées sur le boxtree posé.
pub(crate) struct FindState {
    /// Requête normalisée (minuscules) — la re-coloration
    /// d'une nouvelle passe repart du DOM si le layout a
    /// refondu entre-temps.
    pub(crate) query: String,
    /// Rectangles **par occurrence**, dans l'ordre du
    /// document — un match qui enjambe plusieurs fragments
    /// ou lignes groupe ses sous-rects (compte et navigation
    /// restent à l'occurrence).
    pub(crate) rects: Vec<Vec<Rect>>,
    /// Occurrence courante (mise en valeur plus forte).
    pub(crate) current: usize,
}

/// Occurrences de `query` dans l'arbre — comparaison
/// insensible à la casse, sur le texte tel qu'affiché.
/// Ordre = parcours du boxtree (ordre du document posé).
pub(crate) fn find_text_boxes(bx: &LayoutBox, needle: &str) -> Vec<Vec<Rect>> {
    let mut out = Vec::new();
    if needle.is_empty() {
        return out;
    }
    // Nœuds dont le texte est rendu via des fragments de
    // ligne — collectés globalement : les fragments d'un
    // texte niché (<b> dans <p>) vivent dans les lignes de
    // l'ancêtre, pas à côté de leur boîte `Text` source.
    let mut covered = std::collections::HashSet::new();
    collect_covered(bx, &mut covered);
    find_into(bx, needle, &covered, &mut out);
    out
}

fn collect_covered(bx: &LayoutBox, covered: &mut std::collections::HashSet<explorer_dom::NodeId>) {
    for line in &bx.lines {
        for frag in &line.fragments {
            if let Some(n) = frag.node {
                covered.insert(n);
            }
        }
    }
    for c in &bx.children {
        collect_covered(c, covered);
    }
}

fn find_into(
    bx: &LayoutBox,
    needle: &str,
    covered: &std::collections::HashSet<explorer_dom::NodeId>,
    out: &mut Vec<Vec<Rect>>,
) {
    // R-35 : le haystack = concat des fragments de toutes
    // les lignes de la boîte — une occurrence qui enjambe
    // plusieurs fragments (coupure inline, style interne)
    // ou un retour à la ligne couvre leurs sous-rects
    // respectifs, groupés en une occurrence.
    let mut concat = String::new();
    let mut spans = Vec::new();
    for line in &bx.lines {
        for frag in &line.fragments {
            if frag.atomic || frag.text.is_empty() {
                continue;
            }
            let s = concat.len();
            concat.push_str(&frag.text);
            spans.push((frag, s, concat.len()));
        }
    }
    for (m0, m1) in find_icase(&concat, needle) {
        let mut occ = Vec::new();
        for (frag, s, e) in &spans {
            let (lo, hi) = ((*s).max(m0), (*e).min(m1));
            if lo < hi {
                if let Some(r) = explorer_layout::fragment_sub_rect(frag, lo - s, hi - s) {
                    occ.push(r);
                }
            }
        }
        if !occ.is_empty() {
            out.push(occ);
        }
    }
    for c in &bx.children {
        // Repli : boîte `Text` sans couverture de fragments
        // (valeur d'un contrôle — peinte hors lignes) →
        // surlignage boîte entière, une occurrence par match.
        if let Some(t) = &c.text {
            if !c.node.is_some_and(|n| covered.contains(&n)) {
                for _ in find_icase(t, needle) {
                    out.push(vec![c.rect]);
                }
            }
        }
        find_into(c, needle, covered, out);
    }
}

/// Occurrences `(début, fin)` en offsets octet de `hay` —
/// comparaison insensible à la casse caractère à caractère
/// (pliage ASCII + `to_lowercase` Unicode) ; les bornes
/// renvoyées indexent `hay` — la longueur consommée peut
/// différer de celle de `needle` (cas hors ASCII).
fn find_icase(hay: &str, needle: &str) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    if needle.is_empty() {
        return out;
    }
    let mut i = 0usize;
    while i < hay.len() {
        if let Some(end) = icase_end(hay, i, needle) {
            out.push((i, end));
            i = end;
        } else {
            i += hay[i..].chars().next().map_or(1, char::len_utf8);
        }
    }
    out
}

/// `needle` (déjà pliée en minuscules) matche-t-elle `hay`
/// à l'offset octet `start` ? Renvoie la fin octet couverte.
fn icase_end(hay: &str, start: usize, needle: &str) -> Option<usize> {
    let mut it = hay[start..]
        .char_indices()
        .map(|(j, c)| (start + j, c))
        .peekable();
    for nc in needle.chars() {
        let (_, hc) = it.next()?;
        if !hc.eq_ignore_ascii_case(&nc) && !hc.to_lowercase().eq(std::iter::once(nc)) {
            return None;
        }
    }
    Some(it.peek().map_or(hay.len(), |(j, _)| *j))
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
            .and_then(|occ| occ.first())
            .map(|r| r.y)
        else {
            return;
        };
        let vh = self.viewport.1;
        // Le surlignage est un changement de frame
        // indépendant du déplacement : occurrence déjà dans
        // le viewport → `scroll_to_y` ne repeint pas, on le
        // fait quand même (étape 68 — sinon les `FillRect`
        // restent calculés mais jamais rasterisés).
        let moved = self
            .scroll_to_y(tab, (y - vh / 3.0).max(0.0))
            .unwrap_or(false);
        if !moved {
            let _ = self.repaint(tab);
            self.emit_invalidated(tab);
        }
    }
}
