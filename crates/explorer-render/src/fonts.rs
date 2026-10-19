// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Source de fontes d'infrastructure : [`DirFontSource`]
//! scanne un répertoire de `.ttf`/`.otf` et sert les octets
//! au port `FontSource` — c'est la couche qui touche le
//! système de fichiers (jamais le domaine).
//!
//! Matching simple MVP : le nom de famille demandé est
//! comparé (case-insensitive, espaces ignorés) aux noms
//! lus dans la table `name` de chaque fonte — les
//! génériques CSS (`sans-serif`, `serif`, `monospace`,
//! `cursive`, `fantasy`) mappent sur des familles
//! candidates fournies à la construction. Pas de graisse/
//! style fine pour l'instant (premier fichier qui matche).

use std::collections::HashMap;
use std::path::Path;

use explorer_layout::font::{FontRequest, FontSource};

/// Fonte découverte : familles extraites de la table `name`,
/// chemin vers le fichier.
struct FoundFont {
    families: Vec<String>,
    /// Octets chargés au scan (répertoire borné, MVP).
    data: Vec<u8>,
}

/// Source de fontes adossée à un répertoire local.
pub struct DirFontSource {
    fonts: Vec<FoundFont>,
    /// Générique CSS → chemin résolu à la construction.
    generics: HashMap<String, usize>,
    /// Famille de repli (première fonte trouvée).
    fallback: Option<usize>,
}

impl DirFontSource {
    /// Scanne `dir` (non récursif) : `.ttf`/`.otf` lisibles
    /// par `ttf-parser`. Répertoire absent → source vide
    /// (dégradation, pas d'erreur).
    #[must_use]
    pub fn new(dir: &Path) -> Self {
        let mut fonts = Vec::new();
        if let Ok(entries) = std::fs::read_dir(dir) {
            for e in entries.flatten() {
                let path = e.path();
                let is_font = path
                    .extension()
                    .and_then(|x| x.to_str())
                    .is_some_and(|x| matches!(x.to_ascii_lowercase().as_str(), "ttf" | "otf"));
                if !is_font {
                    continue;
                }
                let Ok(data) = std::fs::read(&path) else {
                    continue;
                };
                let Ok(face) = ttf_parser::Face::parse(&data, 0) else {
                    continue;
                };
                let mut families = Vec::new();
                for n in face.names() {
                    // 1 = family, 16 = typographic family.
                    if matches!(n.name_id, 1 | 16) {
                        if let Some(s) = n.to_string() {
                            families.push(norm(&s));
                        }
                    }
                }
                if !families.is_empty() {
                    fonts.push(FoundFont { families, data });
                }
            }
        }
        let fallback = if fonts.is_empty() { None } else { Some(0) };
        let mut src = Self {
            fonts,
            generics: HashMap::new(),
            fallback,
        };
        src.map_generics();
        src
    }

    /// Mappe les génériques CSS sur la première famille
    /// candidate trouvée (heuristique de plateforme).
    fn map_generics(&mut self) {
        let candidates: &[(&str, &[&str])] = &[
            (
                "sans-serif",
                &[
                    "arial",
                    "segoe ui",
                    "helvetica",
                    "dejavu sans",
                    "liberation sans",
                ],
            ),
            (
                "serif",
                &[
                    "times new roman",
                    "georgia",
                    "dejavu serif",
                    "liberation serif",
                ],
            ),
            (
                "monospace",
                &[
                    "consolas",
                    "courier new",
                    "dejavu sans mono",
                    "liberation mono",
                ],
            ),
        ];
        for (generic, prefs) in candidates {
            for pref in *prefs {
                if let Some(i) = self.index_of(pref) {
                    self.generics.insert((*generic).to_string(), i);
                    break;
                }
            }
        }
        // `cursive`/`fantasy` → repli sur le premier trouvé.
        for g in ["cursive", "fantasy"] {
            if !self.generics.contains_key(g) {
                if let Some(i) = self.fallback {
                    self.generics.insert(g.to_string(), i);
                }
            }
        }
    }

    /// Index d'une famille par nom normalisé.
    fn index_of(&self, name: &str) -> Option<usize> {
        let want = norm(name);
        self.fonts.iter().position(|f| f.families.contains(&want))
    }

    /// Résout la requête : première famille trouvée, sinon
    /// repli (une famille est toujours tentée — le domaine
    /// suppose `sans-serif` en dernier).
    fn resolve(&self, req: &FontRequest) -> Option<&FoundFont> {
        for fam in &req.families {
            let n = norm(fam);
            if let Some(&i) = self.generics.get(&n) {
                return Some(&self.fonts[i]);
            }
            if let Some(i) = self.index_of(fam) {
                return Some(&self.fonts[i]);
            }
        }
        self.fallback.map(|i| &self.fonts[i])
    }
}

impl FontSource for DirFontSource {
    fn font_data(&self, req: &FontRequest) -> Option<Vec<u8>> {
        self.resolve(req).map(|f| f.data.clone())
    }
}

/// Normalisation pour le matching : minuscules, espaces
/// retirés (`"Segoe UI"` == `"segoeui"`).
fn norm(s: &str) -> String {
    s.chars()
        .filter(|c| !c.is_whitespace())
        .flat_map(char::to_lowercase)
        .collect()
}
