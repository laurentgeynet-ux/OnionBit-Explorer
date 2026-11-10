// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! `@font-face` : extraction des descripteurs utiles au
//! chargement de fontes web (étape 46). La cascade ignore
//! ces règles pour le style des éléments ; elles sont
//! conservées avec leur pile de media queries englobantes à
//! la collecte et servies par [`StyleEngine::font_faces`].

use crate::cascade::{media_matches, StyleEngine};
use crate::parser::Declaration;
use crate::StyleEnv;

/// Une source de `src:` — candidates dans l'ordre
/// d'écriture ; la première qui décode gagne.
#[derive(Debug, Clone, PartialEq)]
pub enum FontSrc {
    /// `url(…)` — cible à résoudre contre la page.
    Url(String),
    /// `local("Nom")` — fonte déjà présente localement.
    Local(String),
}

/// Un `@font-face` exploitable.
#[derive(Debug, Clone, PartialEq)]
pub struct FontFace {
    /// `font-family` déclaré (déquoté).
    pub family: String,
    /// Sources `src:` dans l'ordre.
    pub srcs: Vec<FontSrc>,
    /// `font-weight` (1–1000, défaut 400).
    pub weight: u16,
    /// `font-style: italic|oblique` (défaut false).
    pub italic: bool,
    /// `font-display` brut — comportement « block » appliqué
    /// (fetch synchrone avant layout — documenté ADR-0016).
    pub display: String,
    /// `unicode-range` éclaté en paires `(début, fin)`
    /// inclusives — vide = descripteur absent, la face couvre
    /// tout (§4.3 css-fonts). Joker `?` expansé en plage
    /// (`U+4??` → `U+400-4FF`).
    pub unicode_ranges: Vec<(u32, u32)>,
}

/// `unicode-range: U+XXXX[-YYYY], U+Z?` → plages inclusives.
/// `U+4??` = `U+400-4FF` (le joker remplit les digits bas).
/// Entrée invalide/hors borne → plage sautée (dégradation
/// douce — une face sans plage valide reste universelle).
fn parse_unicode_range(raw: &str) -> Vec<(u32, u32)> {
    let mut out = Vec::new();
    for seg in raw.split(',') {
        let t = seg.trim().strip_prefix("U+").unwrap_or(seg.trim());
        let (a, b) = match t.split_once('-') {
            Some((a, b)) => (a.trim(), b.trim()),
            None => (t, t),
        };
        if a.is_empty() || b.is_empty() {
            continue;
        }
        // Joker `?` : chaque `?` = digit hexa libre bas.
        let lo_s = a.replace('?', "0");
        let hi_s = b.replace('?', "F");
        let (Ok(lo), Ok(hi)) = (
            u32::from_str_radix(&lo_s, 16),
            u32::from_str_radix(&hi_s, 16),
        ) else {
            continue;
        };
        if lo <= hi {
            out.push((lo, hi.min(0x10_FFFF)));
        }
    }
    out
}

/// La plage `unicode-range` de la face couvre-t-elle un
/// point de code ? Absence de descripteur → `true` (face
/// universelle). Utilisé pour prioriser le fetch : les
/// éclatements `unicode-range` (Google Fonts sert des
/// dizaines de faces par sous-ensemble d'écriture) font que
/// les faces latines arrivent en fin de feuille — sans
/// priorité, le budget de fetch s'épuise sur des faces qui
/// ne couvrent pas le texte de la page (vol 771 plyr.io :
/// le latin tombait en fallback glyphe → tout en serif).
#[must_use]
pub fn face_covers(face: &FontFace, cp: u32) -> bool {
    face.unicode_ranges.is_empty()
        || face
            .unicode_ranges
            .iter()
            .any(|&(lo, hi)| cp >= lo && cp <= hi)
}

/// Parse une valeur `src:` : segments séparés par virgules —
/// chacun `url(x) [format(…)]` ou `local(x)`. `format()` est
/// ignoré : la détection par signature magique au décodage
/// est plus fiable (les auteurs mentent souvent).
fn parse_src(raw: &str) -> Vec<FontSrc> {
    fn unquote(s: &str) -> &str {
        s.trim().trim_matches(|c| c == '"' || c == '\'')
    }
    let arg = |seg: &str, name: &str| {
        let low = seg.to_ascii_lowercase();
        low.find(name).and_then(|i| {
            let rest = &seg[i + name.len()..];
            rest.find(')').map(|e| unquote(&rest[..e]).to_owned())
        })
    };
    let mut out = Vec::new();
    for seg in raw.split(',') {
        let seg = seg.trim();
        if let Some(u) = arg(seg, "url(") {
            if !u.is_empty() {
                out.push(FontSrc::Url(u));
            }
        } else if let Some(l) = arg(seg, "local(") {
            if !l.is_empty() {
                out.push(FontSrc::Local(l));
            }
        }
    }
    out
}

/// Descripteurs d'un bloc `@font-face` → [`FontFace`] (`None`
/// si `font-family` ou `src` manquant — face inutilisable).
fn to_face(decls: &[Declaration]) -> Option<FontFace> {
    let get = |name: &str| {
        decls
            .iter()
            .find(|d| d.name.eq_ignore_ascii_case(name))
            .map(|d| d.value.trim().to_string())
    };
    let family = get("font-family")?
        .trim_matches(|c| c == '"' || c == '\'')
        .to_string();
    if family.is_empty() {
        return None;
    }
    let srcs = parse_src(&get("src")?);
    if srcs.is_empty() {
        return None;
    }
    let weight = get("font-weight").map_or(400, |v| match v.as_str() {
        "normal" => 400,
        "bold" => 700,
        _ => v.parse::<u16>().unwrap_or(400).clamp(1, 1000),
    });
    let italic = get("font-style").is_some_and(|v| {
        let v = v.to_ascii_lowercase();
        v == "italic" || v == "oblique"
    });
    Some(FontFace {
        family,
        srcs,
        weight,
        italic,
        display: get("font-display").unwrap_or_else(|| "auto".to_string()),
        unicode_ranges: get("unicode-range")
            .map(|v| parse_unicode_range(&v))
            .unwrap_or_default(),
    })
}

impl StyleEngine {
    /// `@font-face` visibles sous `env` (media englobantes
    /// filtrées). Ordre des feuilles, **sauf** que les faces
    /// dont `unicode-range` exclut l'ASCII sont reléguées en
    /// fin : sous budget de fetch borné, les feuilles qui
    /// éclatent par sous-ensemble d'écriture (Google Fonts :
    /// ~126 faces pour Inter, latin en dernier) feraient
    /// sinon enregistrer des faces sans couverture pour le
    /// texte réel — le shaping tombait en fallback par
    /// glyphe (vol 771 plyr.io : texte en serif de repli).
    #[must_use]
    pub fn font_faces(&self, env: &StyleEnv) -> Vec<FontFace> {
        let mut faces: Vec<FontFace> = self
            .font_faces
            .iter()
            .filter(|f| f.media.iter().all(|qs| media_matches(env, qs)))
            .filter_map(|f| to_face(&f.declarations))
            .collect();
        faces.sort_by_key(|f| u8::from(!face_covers(f, 0x41)));
        faces
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Origin;

    #[test]
    fn font_face_basique() {
        let mut e = StyleEngine::default();
        e.add_css(
            "@font-face { font-family: 'Ma Police'; \
             src: url(a.woff2) format(\"woff2\"), url(a.woff) format(\"woff\"); \
             font-weight: 700; font-style: italic; }",
            Origin::Author,
        );
        let faces = e.font_faces(&StyleEnv::default());
        assert_eq!(faces.len(), 1);
        let f = &faces[0];
        assert_eq!(f.family, "Ma Police");
        assert_eq!(f.weight, 700);
        assert!(f.italic);
        assert_eq!(
            f.srcs,
            vec![
                FontSrc::Url("a.woff2".into()),
                FontSrc::Url("a.woff".into()),
            ]
        );
    }

    #[test]
    fn font_face_sans_src_inutilisable() {
        let mut e = StyleEngine::default();
        e.add_css("@font-face { font-family: X; }", Origin::Author);
        assert_eq!(e.font_faces(&StyleEnv::default()).len(), 0);
    }

    #[test]
    fn font_face_media_filtree() {
        let mut e = StyleEngine::default();
        e.add_css(
            "@media print { @font-face { font-family: P; src: url(p.woff); } } \
             @media screen { @font-face { font-family: S; src: url(s.woff); } }",
            Origin::Author,
        );
        let faces = e.font_faces(&StyleEnv::default());
        assert_eq!(faces.len(), 1);
        assert_eq!(faces[0].family, "S");
    }

    #[test]
    fn font_face_local_et_valeurs_hostiles() {
        let mut e = StyleEngine::default();
        e.add_css(
            "@font-face { font-family: 'H;src:url(x'; src: local(\"Locale\"), url(); } \
             @font-face { src: url(sans-famille.woff); }",
            Origin::Author,
        );
        // Famille injectée : conservée telle quelle (pas de
        // parse CSS à récursion) ; url() vide ignorée ; face
        // sans font-family ignorée.
        let faces = e.font_faces(&StyleEnv::default());
        assert_eq!(faces.len(), 1);
        assert_eq!(faces[0].srcs, vec![FontSrc::Local("Locale".into())]);
    }
}
