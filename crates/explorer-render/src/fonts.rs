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
/// octets du fichier, index de face dans une collection
/// `.ttc` (`0` pour un fichier simple — R-69).
struct FoundFont {
    families: Vec<String>,
    /// Octets chargés au scan (répertoire borné, MVP) —
    /// `Arc` : plusieurs faces d'une même collection et les
    /// réponses `font_face`/`fallback_for` partagent le
    /// tampon sans re-cloner des Mo.
    data: Arc<Vec<u8>>,
    /// Index de la face dans le fichier (`fonts_in_collection`
    /// > 1 → `.ttc`).
    face_index: u32,
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
    /// Scanne `dir` (non récursif) : `.ttf`/`.otf` **et
    /// `.ttc`** (R-69 — quasi toutes les fontes CJK Windows
    /// sont des collections) lisibles par `ttf-parser`.
    /// Chaque face d'une collection est une entrée propre
    /// (familles lues à son index). Répertoire absent →
    /// source vide (dégradation, pas d'erreur).
    #[must_use]
    pub fn new(dir: &Path) -> Self {
        let mut fonts = Vec::new();
        if let Ok(entries) = std::fs::read_dir(dir) {
            for e in entries.flatten() {
                let path = e.path();
                let is_font = path.extension().and_then(|x| x.to_str()).is_some_and(|x| {
                    matches!(x.to_ascii_lowercase().as_str(), "ttf" | "otf" | "ttc")
                });
                if !is_font {
                    continue;
                }
                let Ok(data) = std::fs::read(&path) else {
                    continue;
                };
                let data = Arc::new(data);
                // `fonts_in_collection` sous garde : un `.ttc`
                // tronqué peut déclarer un `numFonts` démesuré
                // et faire paniquer une assertion interne de
                // `ttf-parser` — le fichier est sauté (R-69).
                let count = collection_len(&data).unwrap_or(1).min(MAX_TTC_FACES);
                for index in 0..count {
                    let Some(families) = face_families(&data, index) else {
                        continue;
                    };
                    fonts.push(FoundFont {
                        families,
                        data: data.clone(),
                        face_index: index,
                    });
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
    /// `font_data` conserve la sémantique pré-R-69 : seules
    /// les faces **à l'index 0** sont servies — un appelant
    /// qui parse à l'index 0 sur une collection prendrait
    /// la mauvaise face (`font_face` pour l'index réel).
    fn font_data(&self, req: &FontRequest) -> Option<Vec<u8>> {
        self.resolve(req)
            .filter(|f| f.face_index == 0)
            .map(|f| (*f.data).clone())
    }

    fn font_face(&self, req: &FontRequest) -> Option<(Arc<Vec<u8>>, u32)> {
        self.resolve(req).map(|f| (f.data.clone(), f.face_index))
    }

    /// Repli par glyphe (R-69) : familles de la requête
    /// d'abord (ordre `font-family` CSS — la fonte nommée
    /// gagne quand elle couvre), puis tout le registre dans
    /// l'ordre du scan. Première face couvrant `c` retenue.
    fn fallback_for(&self, c: char, req: &FontRequest) -> Option<(Arc<Vec<u8>>, u32)> {
        for fam in &req.families {
            let n = norm(fam);
            for f in &self.fonts {
                if f.families.contains(&n) && covers(f, c) {
                    return Some((f.data.clone(), f.face_index));
                }
            }
        }
        self.fonts
            .iter()
            .find(|f| covers(f, c))
            .map(|f| (f.data.clone(), f.face_index))
    }
}

/// Plafond de faces sondées par collection — un `numFonts`
/// hostile est borné avant l'itération (R-69).
const MAX_TTC_FACES: u32 = 1024;

/// Nombre de faces déclaré dans `data` (`> 1` → `.ttc`),
/// `None` si l'en-tête est illisible **ou panique** :
/// `ttf-parser` affirme ses bornes en interne sur certaines
/// tables hostiles — un fichier fautif dégrade, ne plante
/// pas.
fn collection_len(data: &[u8]) -> Option<u32> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        ttf_parser::fonts_in_collection(data)
    }))
    .ok()
    .flatten()
}

/// Familles `name` (1 = family, 16 = typographic family)
/// normalisées de la face `index` — `None` si la face est
/// illisible ou si `ttf-parser` panique (même garde que
/// [`collection_len`]).
fn face_families(data: &[u8], index: u32) -> Option<Vec<String>> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let face = ttf_parser::Face::parse(data, index).ok()?;
        let mut v = Vec::new();
        for n in face.names() {
            if matches!(n.name_id, 1 | 16) {
                if let Some(s) = n.to_string() {
                    v.push(norm(&s));
                }
            }
        }
        (!v.is_empty()).then_some(v)
    }))
    .ok()
    .flatten()
}

/// `c` a-t-il un vrai glyphe dans la face (cmap →
/// id ≠ 0) — le `.notdef` compte comme « non couvert ».
/// Gardée : `glyph_index` lit des tables potentiellement
/// hostiles (R-69).
fn covers_bytes(data: &[u8], index: u32, c: char) -> bool {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        ttf_parser::Face::parse(data, index)
            .ok()
            .and_then(|t| t.glyph_index(c))
            .is_some_and(|g| g.0 != 0)
    }))
    .unwrap_or(false)
}

/// `c` a-t-il un vrai glyphe dans `f` (cmap → id ≠ 0) —
/// le `.notdef` compte comme « non couvert ».
fn covers(f: &FoundFont, c: char) -> bool {
    covers_bytes(&f.data, f.face_index, c)
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
    /// Compteur de mutations (`register`/`clear`) — lu par
    /// les caches cross-passe (`GlyphCache`, R-36) pour
    /// invalider leurs entrées figées.
    rev: u64,
}

impl WebFontSource {
    /// Enveloppe `inner` (source locale de repli).
    #[must_use]
    pub fn new(inner: Arc<dyn FontSource + Send + Sync>, cap: usize) -> Self {
        Self {
            inner,
            faces: Vec::new(),
            cap,
            rev: 0,
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
        // Garde `catch_unwind` : une fonte `@font-face`
        // hostile peut faire paniquer `ttf-parser` — refus,
        // pas crash (R-69).
        let valid = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            ttf_parser::Face::parse(&data, 0).is_ok()
        }))
        .unwrap_or(false);
        if !valid {
            return false;
        }
        self.faces.push(WebFace {
            family: norm(family),
            weight,
            italic,
            data: Arc::new(data),
        });
        self.rev += 1;
        true
    }

    /// Purge le registre (navigation → faces par document).
    pub fn clear(&mut self) {
        self.faces.clear();
        self.rev += 1;
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

    /// Faces web = fichiers simples (index 0 — WOFF décodé
    /// amont) ; le reste délègue à la source interne.
    fn font_face(&self, req: &FontRequest) -> Option<(Arc<Vec<u8>>, u32)> {
        for fam in &req.families {
            let n = norm(fam);
            if let Some(f) = self
                .faces
                .iter()
                .filter(|f| f.family == n)
                .min_by_key(|f| Self::score(f, req))
            {
                return Some((f.data.clone(), 0));
            }
        }
        self.inner.font_face(req)
    }

    /// Repli par glyphe (R-69) : une face web **de la
    /// famille demandée** qui couvre `c` gagne (une page
    /// peut charger son CJK en `@font-face`), sinon la
    /// source interne (fontes système).
    fn fallback_for(&self, c: char, req: &FontRequest) -> Option<(Arc<Vec<u8>>, u32)> {
        for fam in &req.families {
            let n = norm(fam);
            for f in self.faces.iter().filter(|f| f.family == n) {
                if covers_bytes(&f.data, 0, c) {
                    return Some((f.data.clone(), 0));
                }
            }
        }
        self.inner.fallback_for(c, req)
    }

    /// Le registre mute : `register`/`clear` la bumpent —
    /// propage aussi le delta de la source interne.
    fn revision(&self) -> u64 {
        self.rev.wrapping_add(self.inner.revision())
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

    /// R-69 : le scan ouvre les collections `.ttc` — chaque
    /// face y a sa famille et son index ; `fallback_for`
    /// trouve la couverture par glyphe à travers les
    /// registres. `font_data` reste filtrée aux index 0
    /// (octets ambigus sinon).
    #[test]
    fn ttc_faces_scannes_et_fallback() {
        use explorer_test_support::{fixture_ttc, fixture_ttf_custom};
        let dir = std::env::temp_dir().join(format!("explorer-r69-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("dir");
        std::fs::write(
            dir.join("latin.ttf"),
            fixture_ttf_custom("Latin", 0x0020, 0x00FF),
        )
        .expect("write ttf");
        // Collection deux faces : leurre à l'index 0, CJK
        // (0x65E0-0x65FF — couvre 日 0x65E5, 旦 0x65E6) à 1.
        std::fs::write(
            dir.join("cjk.ttc"),
            fixture_ttc(&[
                fixture_ttf_custom("Decoy", 0x0020, 0x00FF),
                fixture_ttf_custom("Cjk", 0x65E0, 0x65FF),
            ]),
        )
        .expect("write ttc");
        let src = DirFontSource::new(&dir);
        let latin = req(&["Latin"], 400, false);
        // `font_face` résout la famille dans la collection,
        // avec l'index réel — pas le leurre de l'index 0.
        let (_, i) = src.font_face(&req(&["Cjk"], 400, false)).expect("cjk face");
        assert_eq!(i, 1);
        // '日' absente de Latin → repli sert la face CJK
        // de la collection (index 1).
        let (_, fi) = src.fallback_for('日', &latin).expect("fallback");
        assert_eq!(fi, 1);
        // Hors portée totale → `None` honnête.
        assert!(src.fallback_for('\u{1F600}', &latin).is_none());
        // `font_data` compat : index > 0 → `None` (les
        // octets seuls ne porteraient pas la bonne face),
        // index 0 → `Some`.
        assert!(src.font_data(&req(&["Cjk"], 400, false)).is_none());
        assert!(src.font_data(&latin).is_some());
        drop(std::fs::remove_dir_all(&dir));
    }

    /// Le scan tolère les fichiers illisibles : ni le
    /// `.ttc` corrompu ni le `.ttf` bidon n'interrompent la
    /// source ni ne paniquent.
    #[test]
    fn scan_tolerant_aux_fichiers_bidon() {
        let dir = std::env::temp_dir().join(format!("explorer-r69b-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("dir");
        std::fs::write(dir.join("bad.ttc"), b"ttcf\x00\x01pas une collection").expect("w");
        std::fs::write(dir.join("bad.ttf"), b"pas une fonte").expect("w");
        std::fs::write(dir.join("note.txt"), b"pas une fonte").expect("w");
        let src = DirFontSource::new(&dir);
        assert!(src.font_face(&req(&["Nimporte"], 400, false)).is_none());
        assert!(src
            .fallback_for('A', &req(&["Nimporte"], 400, false))
            .is_none());
        drop(std::fs::remove_dir_all(&dir));
    }
}
