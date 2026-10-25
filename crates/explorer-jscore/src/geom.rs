// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Instantané de géométrie publié par l'embedder (issue #19, JS-8).
//!
//! Le moteur ne connaît pas la mise en page : la session marche
//! l'arbre `LayoutBox` commis (le seul état cohérent — jamais un
//! arbre en mutation) et remplit une table `element_path →
//! union des boîtes`, lue en direct par
//! `getBoundingClientRect`/`offset*`/`client*`/`scroll*`.
//!
//! Le partage se fait par [`GeometrySlot`] (`Arc<Mutex<..>>`), pas par
//! appel synchrone : le JS tourne sur son worker, aucun
//! aller-retour vers la session n'est possible pendant
//! l'exécution — la géométrie est un **état publié**, pas une
//! requête.
//!
//! Bornes assumées :
//!
//! - document top-level seul — les contextes de cadre partagent
//!   la table (leurs `element_path` calculés sur leur propre
//!   miroir manquent → 0, comportement antérieur) ;
//! - union des boîtes du nœud ≈ border-box (`getBoundingClientRect`
//!   spec joint les fragments en-ligne — même agrégat) ;
//! - instantané du dernier commit paint : un script qui mute le
//!   DOM puis lit une mesure dans le même tour obtient la mesure
//!   d'avant-mutation (pas de reflow synchrone — la resynchro
//!   arrive à l'apply suivant) ;
//! - `client*`/`scroll*` ≈ même boîte (padding/content-box non
//!   distinguées ici) ; `scroll*` de l'élément racine =
//!   dimensions du document publiées (`doc_w`/`doc_h`).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// Rectangle en px CSS.
#[derive(Debug, Clone, Copy, Default)]
pub struct GeoRect {
    /// Abscisse.
    pub x: f32,
    /// Ordonnée.
    pub y: f32,
    /// Largeur.
    pub w: f32,
    /// Hauteur.
    pub h: f32,
}

impl GeoRect {
    /// Union couvrante de deux rects (fragments d'un même
    /// élément) — `self` vide (`w`/`h` ≤ 0) adopte `other`.
    #[must_use]
    pub fn union(self, other: Self) -> Self {
        if self.w <= 0.0 || self.h <= 0.0 {
            return other;
        }
        if other.w <= 0.0 || other.h <= 0.0 {
            return self;
        }
        let x = self.x.min(other.x);
        let y = self.y.min(other.y);
        Self {
            x,
            y,
            w: (self.x + self.w).max(other.x + other.w) - x,
            h: (self.y + self.h).max(other.y + other.h) - y,
        }
    }
}

/// Géométrie d'un élément du document.
#[derive(Debug, Clone, Copy, Default)]
pub struct ElementGeom {
    /// Union des boîtes du nœud — coords **document**, sauf si
    /// `fixed` (alors déjà coords viewport : le layout pose
    /// `position:fixed` contre le viewport — i#18).
    pub rect: GeoRect,
    /// Une boîte contributrice vit sous un ancêtre
    /// `position:fixed` — le rect est en coords viewport
    /// (ne pas soustraire le scroll à la lecture).
    pub fixed: bool,
    /// Une boîte contributrice est **positionnée** (computed
    /// `position` ≠ `static` — relative/absolute/fixed/sticky,
    /// JS-C97 palier 2) : sert au calcul d'`offsetParent`
    /// (plus proche ancêtre positionné — le miroir marche sa
    /// propre chaîne et consulte ce bit par `element_path`).
    pub positioned: bool,
}

/// Table publiée — reconstruite entièrement par l'embedder à
/// chaque commit paint (nav, apply, relayout, restauration) :
/// jamais de mesure sur arbre en mutation, et un état périmé
/// ne survit qu'à la fenêtre détachée (i#23 — borné).
#[derive(Debug, Default)]
pub struct GeometryMap {
    /// `element_path` (`tag`, index parmi les frères éléments de
    /// même tag — monnaie `DomMutation`, calculable des deux
    /// côtés dès le parse) → géométrie union du nœud.
    pub rects: HashMap<Vec<(String, u32)>, ElementGeom>,
    /// Scroll courant — à soustraire aux coords document pour
    /// passer en coords viewport (`fixed` : déjà dedans).
    /// Mis à jour sans rebuild au scroll input.
    pub scroll_x: f32,
    /// Idem, axe vertical.
    pub scroll_y: f32,
    /// Largeur du document (`documentElement.scrollWidth` ≈
    /// largeur du conteneur racine).
    pub doc_w: f32,
    /// Hauteur du document (`documentElement.scrollHeight` —
    /// le `doc_h` de session).
    pub doc_h: f32,
}

/// Référence partagée session ↔ moteur. `Arc` (pas `Rc`) : la
/// map est écrite sur le thread session et lue sur le worker
/// JS — `Mutex` standard, jamais gardé pendant un `.await`.
pub type GeometrySlot = Arc<Mutex<GeometryMap>>;
