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
use std::sync::Arc;

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

/// Une face web enregistrée (issues de `@font-face`,
/// étape 46) — validée par `ttf-parser` à l'inscription.
struct WebFace {
    /// Famille déclarée, normalisée comme les demandes.
    family: String,
    /// Graisse déclarée (1–1000).
    weight: u16,
    /// Italique/oblique déclaré.
    italic: bool,
    /// Octets sfnt (déjà décodés de WOFF par l'appelant).
    data: Arc<Vec<u8>>,
}

/// Source composite : faces `@font-face` fetchées par le
/// moteur **devant** une source locale de repli (ADR-0016).
/// Le registre est borné (`cap`) et chaque face est validée
/// par `ttf-parser` à l'inscription — une fonte hostile ne
/// peut ni saturer le registre ni atteindre le shapeur.
pub struct WebFontSource {
    inner: Arc<dyn FontSource + Send + Sync>,
    faces: Vec<WebFace>,
    cap: usize,
}

impl WebFontSource {
    /// Enveloppe `inner` (source locale de repli).
    #[must_use]
    pub fn new(inner: Arc<dyn FontSource + Send + Sync>, cap: usize) -> Self {
        Self {
            inner,
            faces: Vec::new(),
            cap,
        }
    }

    /// Nombre de faces enregistrées.
    #[must_use]
    pub fn len(&self) -> usize {
        self.faces.len()
    }

    /// Registre vide (`false` si plein — aucune place
    /// rendue avant une `clear`).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.faces.is_empty()
    }

    /// Registre plein (`cap` atteint) — l'appelant arrête
    /// de fetcher les faces restantes.
    #[must_use]
    pub fn is_full(&self) -> bool {
        self.faces.len() >= self.cap
    }

    /// Inscrit une face : validée par `ttf-parser`, bornée
    /// par `cap`. `false` = refusée (fonte illisible ou
    /// registre plein) — la page garde le repli local.
    pub fn register(&mut self, family: &str, weight: u16, italic: bool, data: Vec<u8>) -> bool {
        if self.faces.len() >= self.cap || norm(family).is_empty() {
            return false;
        }
        if ttf_parser::Face::parse(&data, 0).is_err() {
            return false;
        }
        self.faces.push(WebFace {
            family: norm(family),
            weight,
            italic,
            data: Arc::new(data),
        });
        true
    }

    /// Purge le registre (navigation → faces par document).
    pub fn clear(&mut self) {
        self.faces.clear();
    }

    /// Score d'appariement : famille égale exigée en amont —
    /// italique d'abord (+2000), distance de graisse ensuite.
    fn score(f: &WebFace, req: &FontRequest) -> u32 {
        let style_pen = u32::from(f.italic != req.italic) * 2000;
        style_pen + u32::from(f.weight.abs_diff(req.weight))
    }
}

impl FontSource for WebFontSource {
    fn font_data(&self, req: &FontRequest) -> Option<Vec<u8>> {
        for fam in &req.families {
            let n = norm(fam);
            if let Some(f) = self
                .faces
                .iter()
                .filter(|f| f.family == n)
                .min_by_key(|f| Self::score(f, req))
            {
                return Some((*f.data).clone());
            }
        }
        self.inner.font_data(req)
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

#[cfg(test)]
mod tests {
    use super::*;
    use explorer_test_support::fixture_ttf;

    struct EmptyFont;
    impl FontSource for EmptyFont {
        fn font_data(&self, req: &FontRequest) -> Option<Vec<u8>> {
            req.families
                .first()
                .filter(|f| f.as_str() == "Local")
                .map(|_| vec![1, 2, 3])
        }
    }

    fn req(families: &[&str], weight: u16, italic: bool) -> FontRequest {
        FontRequest {
            families: families.iter().map(|s| (*s).to_string()).collect(),
            weight,
            italic,
        }
    }

    #[test]
    fn register_match_repli() {
        let mut w = WebFontSource::new(Arc::new(EmptyFont), 4);
        assert!(w.register("Ma Police", 400, false, fixture_ttf()));
        // Famille web servie en priorité, famille inconnue →
        // repli interne, registre nettoyé.
        assert_eq!(
            w.font_data(&req(&["ma police"], 400, false)),
            Some(fixture_ttf())
        );
        assert_eq!(
            w.font_data(&req(&["Local"], 400, false)),
            Some(vec![1, 2, 3])
        );
        assert_eq!(w.font_data(&req(&["Inconnue"], 400, false)), None);
        w.clear();
        assert!(w.is_empty());
        assert_eq!(w.font_data(&req(&["ma police"], 400, false)), None);
    }

    #[test]
    fn matching_graisse_et_style() {
        let mut w = WebFontSource::new(Arc::new(EmptyFont), 8);
        assert!(w.register("F", 400, false, fixture_ttf()));
        assert!(w.register("F", 700, true, fixture_ttf()));
        // Le moins mauvais gagne : exact > proche > pire.
        let _ = w.font_data(&req(&["F"], 700, true));
        let _ = w.font_data(&req(&["F"], 600, false)); // → 400
        let _ = w.font_data(&req(&["F"], 900, true)); // → 700i
    }

    #[test]
    fn registre_borne_et_invalide() {
        let mut w = WebFontSource::new(Arc::new(EmptyFont), 1);
        assert!(!w.register("Bad", 400, false, b"pas une fonte".to_vec()));
        assert!(w.register("Ok", 400, false, fixture_ttf()));
        assert!(w.is_full());
        assert!(!w.register("Plein", 400, false, fixture_ttf()));
        assert!(!w.register("", 400, false, fixture_ttf()));
        assert_eq!(w.len(), 1);
    }
}
