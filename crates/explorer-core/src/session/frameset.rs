// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! `<frameset>`/`<frame>` — tranche legacy du journal
//! inter-équipes 2026-11-15, point (a) : « grille
//! d'iframes ». Le parseur pose l'arbre correctement
//! (mode `InFrameset` html5lib) mais le boxtree ignore
//! les deux balises — un document frameset peignait
//! une page blanche.
//!
//! Réécriture DOM post-parse, pure session, **aucun
//! changement de contrat PIPE** : chaque `<frame>`
//! devient un `<iframe>` positionné absolument dans
//! un `<div>` grille (calcul de grille `rows`/`cols`
//! côté NAV, en px résolus sur le viewport), et la
//! machinerie iframe existante — collecte, fetch,
//! sous-contextes, composite, bfcache — les prend
//! sans aucune branche dédiée.
//!
//! Sémantique (HTML4 §16, tolérante) :
//! - `rows`/`cols` absents = `"100%"` ; longueurs
//!   `N` px, `N%` du conteneur, `N*` poids relatif
//!   (`*` seul = `1*`) — les `*` se partagent le
//!   reste après px et % ; entrée invalide = `1*` ;
//! - le produit lignes × colonnes fait la grille :
//!   `<frame>` surnuméraires ignorées, cellules
//!   vides = espace vide ;
//! - `<noframes>` est sauté (les cadres sont
//!   supportés — son contenu n'existe que pour les
//!   clients qui refusent les cadres) ;
//! - attributs d'intégration copiés : `src`, `name`
//!   (cible de liens `target`), `scrolling`,
//!   `frameborder` (`0` → `border:0`),
//!   `marginwidth`/`marginheight`, `noresize`,
//!   `longdesc` — pas de resize ni de bordures
//!   draggables : dégradation honnête, jamais de
//!   panic sur specs hostiles.

use explorer_dom::{Dom, NodeId};

use super::attr_opt;

/// Longueur de grille `rows`/`cols` — `N` pixels,
/// `N%` du conteneur, `N*` poids relatif.
#[derive(Clone, Copy)]
enum Len {
    Px(f32),
    Pct(f32),
    Fr(f32),
}

/// `rows="100,30%,2*,*"` → longueurs — entrée vide ou
/// inintelligible = `1*` (part relative, tolérant).
fn parse_lens(spec: &str) -> Vec<Len> {
    spec.split(',')
        .map(|raw| {
            let t = raw.trim();
            if let Some(p) = t.strip_suffix('%') {
                p.trim().parse::<f32>().map_or(Len::Fr(1.0), Len::Pct)
            } else if let Some(f) = t.strip_suffix('*') {
                let w = if f.trim().is_empty() {
                    1.0
                } else {
                    f.trim().parse::<f32>().unwrap_or(1.0)
                };
                Len::Fr(w.max(0.0))
            } else {
                t.parse::<f32>().map_or(Len::Fr(1.0), Len::Px)
            }
        })
        .collect()
}

/// Résout les longueurs en px dans `total` : px et %
/// d'abord, le reste réparti au poids des `*`
/// (reste négatif clampé à 0 — une spec qui dépasse
/// le conteneur n'inverse jamais une cellule).
fn resolve_lens(lens: &[Len], total: f32) -> Vec<f32> {
    let mut out = vec![0.0_f32; lens.len()];
    let mut fr = 0.0_f32;
    for (i, l) in lens.iter().enumerate() {
        match *l {
            Len::Px(px) => out[i] = px.max(0.0),
            Len::Pct(p) => out[i] = (total * p.max(0.0) / 100.0).max(0.0),
            Len::Fr(w) => fr += w,
        }
    }
    let rem = (total - out.iter().sum::<f32>()).max(0.0);
    for (i, l) in lens.iter().enumerate() {
        if let Len::Fr(w) = *l {
            out[i] = if fr > 0.0 { rem * w / fr } else { 0.0 };
        }
    }
    out
}

/// `position:absolute` + boîte — style inline synthétique,
/// aucune feuille externe à charger.
fn abs_style(x: f32, y: f32, w: f32, h: f32, extra: &str) -> String {
    format!("position:absolute;left:{x}px;top:{y}px;width:{w}px;height:{h}px{extra}")
}

/// `<iframe>` synthétique d'une `<frame>` — attributs
/// d'intégration copiés, `frameborder="0"` → `border:0`.
fn frame_to_iframe(dom: &mut Dom, f: NodeId, x: f32, y: f32, w: f32, h: f32) -> Option<NodeId> {
    let ifr = dom.create_element("iframe").ok()?;
    // Attributs copiés en `String` d'abord — l'emprunt
    // `attr_opt` ne peut cohabiter avec `set_attribute`.
    let attrs: Vec<(&'static str, String)> = [
        "src",
        "name",
        "scrolling",
        "marginwidth",
        "marginheight",
        "noresize",
        "longdesc",
    ]
    .into_iter()
    .filter_map(|a| attr_opt(dom, f, a).map(|v| (a, v.to_owned())))
    .collect();
    for (a, v) in attrs {
        let _ = dom.set_attribute(ifr, a, &v);
    }
    let border = if attr_opt(dom, f, "frameborder").is_some_and(|v| v.trim() == "0") {
        ";border:0"
    } else {
        ""
    };
    let _ = dom.set_attribute(ifr, "style", &abs_style(x, y, w, h, border));
    Some(ifr)
}

/// `<div>` grille positionné à `(x, y, w, h)` —
/// cellules remplies dans l'ordre par les `<frame>`
/// (→ `<iframe>`) et `<frameset>` imbriqués (→ sous-
/// grilles récursives). `None` si le `<frameset>` n'a
/// aucun enfant cadre.
fn build_grid(dom: &mut Dom, fs: NodeId, x: f32, y: f32, w: f32, h: f32) -> Option<NodeId> {
    let rows = attr_opt(dom, fs, "rows").unwrap_or("100%");
    let cols = attr_opt(dom, fs, "cols").unwrap_or("100%");
    let rh = resolve_lens(&parse_lens(rows), h);
    let cw = resolve_lens(&parse_lens(cols), w);
    let Ok(kids) = dom.children(fs) else {
        return None;
    };
    // Collecté avant les mutations — l'itérateur paresseux
    // garderait `dom` emprunté pendant `append_child`.
    let cells: Vec<NodeId> = kids
        .into_iter()
        .filter(|n| {
            dom.tag_name(*n)
                .is_ok_and(|t| t == "frame" || t == "frameset")
        })
        .collect();
    let mut cells = cells.into_iter();
    let mut filled = false;
    let grid = dom.create_element("div").ok()?;
    let _ = dom.set_attribute(grid, "style", &abs_style(x, y, w, h, ""));
    let mut cy = y;
    for ch in &rh {
        let mut cx = x;
        for cwid in &cw {
            let Some(child) = cells.next() else {
                return filled.then_some(grid);
            };
            let cell = if dom.tag_name(child).is_ok_and(|t| t == "frameset") {
                build_grid(dom, child, cx, cy, *cwid, *ch)
            } else {
                frame_to_iframe(dom, child, cx, cy, *cwid, *ch)
            };
            if let Some(c) = cell {
                let _ = dom.append_child(grid, c);
                filled = true;
            }
            cx += cwid;
        }
        cy += ch;
    }
    filled.then_some(grid)
}

/// Réécrit les `<frameset>` enfants de `<html>` en
/// grilles d'`<iframe>` — no-op sur un document
/// classique (aucun frameset, 99 % des pages).
/// `viewport` sert la résolution px des `rows`/`cols`
/// (un frameset occupe toute la fenêtre par spec).
pub(crate) fn expand_framesets(dom: &mut Dom, viewport: (f32, f32)) {
    let Some(html_el) = dom.document_element() else {
        return;
    };
    let Ok(kids) = dom.children(html_el) else {
        return;
    };
    for fs in kids {
        if !dom.tag_name(fs).is_ok_and(|t| t == "frameset") {
            continue;
        }
        if let Some(grid) = build_grid(dom, fs, 0.0, 0.0, viewport.0, viewport.1) {
            let _ = dom.insert_before(html_el, grid, Some(fs));
        }
        // Le frameset source sort de l'arbre — ses `<frame>`
        // ont été relues en attributs, le boxtree ne doit
        // pas le voir (pas de boîte dédiée côté PIPE).
        let _ = dom.remove(fs);
    }
}
