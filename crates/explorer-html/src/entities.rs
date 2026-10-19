// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Références de caractères nommées (`&amp;`, `&nbsp;`…).
//!
//! La spec complète en définit ~2 200 ; cette table couvre le jeu
//! Latin-1 (autorisé sans point-virgule dans le contenu par
//! compatibilité historique) + les entités courantes du web réel.
//! Elle est conçue pour être remplacée par une table générée
//! complète sans changement d'API (corpus de conformité).

/// Une entité nommée : nom sans `&` ni `;`, rendu, et `legacy` =
/// autorisée sans `;` dans le contenu (jamais dans les attributs
/// suivis de `=`/alphanumérique — règle de l'esperluette ambiguë).
struct NamedRef {
    name: &'static str,
    value: &'static str,
    legacy: bool,
}

macro_rules! refs {
    ($($name:literal => $value:literal : $legacy:tt),* $(,)?) => {
        &[
            $(NamedRef { name: $name, value: $value, legacy: $legacy }),*
        ]
    };
}

/// Table des entités — triée par nom (aide visuelle ; la recherche
/// est un scan de plus long préfixe, la table est courte).
static NAMED_REFS: &[NamedRef] = refs![
    // --- Latin-1 legacy (sans `;` toléré dans le contenu) ---
    "AElig" => "\u{C6}" : true,  "Aacute" => "\u{C1}" : true,
    "Acirc" => "\u{C2}" : true,  "Agrave" => "\u{C0}" : true,
    "Aring" => "\u{C5}" : true,  "Atilde" => "\u{C3}" : true,
    "Auml" => "\u{C4}" : true,   "COPY" => "\u{A9}" : true,
    "Ccedil" => "\u{C7}" : true, "ETH" => "\u{D0}" : true,
    "Eacute" => "\u{C9}" : true, "Ecirc" => "\u{CA}" : true,
    "Egrave" => "\u{C8}" : true, "Euml" => "\u{CB}" : true,
    "GT" => ">" : true,          "Iacute" => "\u{CD}" : true,
    "Icirc" => "\u{CE}" : true,  "Igrave" => "\u{CC}" : true,
    "Iuml" => "\u{CF}" : true,   "LT" => "<" : true,
    "Ntilde" => "\u{D1}" : true, "Oacute" => "\u{D3}" : true,
    "Ocirc" => "\u{D4}" : true,  "Ograve" => "\u{D2}" : true,
    "Oslash" => "\u{D8}" : true, "Otilde" => "\u{D5}" : true,
    "Ouml" => "\u{D6}" : true,   "QUOT" => "\"" : true,
    "REG" => "\u{AE}" : true,    "THORN" => "\u{DE}" : true,
    "Uacute" => "\u{DA}" : true, "Ucirc" => "\u{DB}" : true,
    "Ugrave" => "\u{D9}" : true, "Uuml" => "\u{DC}" : true,
    "Yacute" => "\u{DD}" : true, "aacute" => "\u{E1}" : true,
    "acirc" => "\u{E2}" : true,  "acute" => "\u{B4}" : true,
    "aelig" => "\u{E6}" : true,  "agrave" => "\u{E0}" : true,
    "amp" => "&" : true,         "aring" => "\u{E5}" : true,
    "atilde" => "\u{E3}" : true, "auml" => "\u{E4}" : true,
    "brvbar" => "\u{A6}" : true, "ccedil" => "\u{E7}" : true,
    "cedil" => "\u{B8}" : true,  "cent" => "\u{A2}" : true,
    "copy" => "\u{A9}" : true,   "curren" => "\u{A4}" : true,
    "deg" => "\u{B0}" : true,    "divide" => "\u{F7}" : true,
    "eacute" => "\u{E9}" : true, "ecirc" => "\u{EA}" : true,
    "egrave" => "\u{E8}" : true, "eth" => "\u{F0}" : true,
    "euml" => "\u{EB}" : true,   "frac12" => "\u{BD}" : true,
    "frac14" => "\u{BC}" : true, "frac34" => "\u{BE}" : true,
    "gt" => ">" : true,          "iacute" => "\u{ED}" : true,
    "icirc" => "\u{EE}" : true,  "iexcl" => "\u{A1}" : true,
    "igrave" => "\u{EC}" : true, "iquest" => "\u{BF}" : true,
    "iuml" => "\u{EF}" : true,   "laquo" => "\u{AB}" : true,
    "lt" => "<" : true,          "macr" => "\u{AF}" : true,
    "micro" => "\u{B5}" : true,  "middot" => "\u{B7}" : true,
    "nbsp" => "\u{A0}" : true,   "not" => "\u{AC}" : true,
    "ntilde" => "\u{F1}" : true, "oacute" => "\u{F3}" : true,
    "ocirc" => "\u{F4}" : true,  "ograve" => "\u{F2}" : true,
    "ordf" => "\u{AA}" : true,   "ordm" => "\u{BA}" : true,
    "oslash" => "\u{F8}" : true, "otilde" => "\u{F5}" : true,
    "ouml" => "\u{F6}" : true,   "para" => "\u{B6}" : true,
    "plusmn" => "\u{B1}" : true, "pound" => "\u{A3}" : true,
    "quot" => "\"" : true,       "raquo" => "\u{BB}" : true,
    "reg" => "\u{AE}" : true,    "sect" => "\u{A7}" : true,
    "shy" => "\u{AD}" : true,    "sup1" => "\u{B9}" : true,
    "sup2" => "\u{B2}" : true,   "sup3" => "\u{B3}" : true,
    "szlig" => "\u{DF}" : true,  "thorn" => "\u{FE}" : true,
    "times" => "\u{D7}" : true,  "uacute" => "\u{FA}" : true,
    "ucirc" => "\u{FB}" : true,  "ugrave" => "\u{F9}" : true,
    "uml" => "\u{A8}" : true,    "uuml" => "\u{FC}" : true,
    "yacute" => "\u{FD}" : true, "yen" => "\u{A5}" : true,
    "yuml" => "\u{FF}" : true,
    // --- Courantes, `;` exigé ---
    "AMP" => "&" : false,        "Alpha" => "\u{391}" : false,
    "Beta" => "\u{392}" : false, "Chi" => "\u{3A7}" : false,
    "Dagger" => "\u{2021}" : false, "Delta" => "\u{394}" : false,
    "Epsilon" => "\u{395}" : false, "Eta" => "\u{397}" : false,
    "Gamma" => "\u{393}" : false, "Iota" => "\u{399}" : false,
    "Kappa" => "\u{39A}" : false, "Lambda" => "\u{39B}" : false,
    "Mu" => "\u{39C}" : false,   "Nu" => "\u{39D}" : false,
    "Omega" => "\u{3A9}" : false, "Omicron" => "\u{39F}" : false,
    "Phi" => "\u{3A6}" : false,  "Pi" => "\u{3A0}" : false,
    "Prime" => "\u{2033}" : false, "Psi" => "\u{3A8}" : false,
    "Rho" => "\u{3A1}" : false,  "Sigma" => "\u{3A3}" : false,
    "Tau" => "\u{3A4}" : false,  "Theta" => "\u{398}" : false,
    "Upsilon" => "\u{3A5}" : false, "Xi" => "\u{39E}" : false,
    "Zeta" => "\u{396}" : false, "alefsym" => "\u{2135}" : false,
    "alpha" => "\u{3B1}" : false, "and" => "\u{2227}" : false,
    "ang" => "\u{2220}" : false, "apos" => "'" : false,
    "asymp" => "\u{2248}" : false, "bdquo" => "\u{201E}" : false,
    "beta" => "\u{3B2}" : false, "bull" => "\u{2022}" : false,
    "cap" => "\u{2229}" : false, "chi" => "\u{3C7}" : false,
    "clubs" => "\u{2663}" : false, "cong" => "\u{2245}" : false,
    "cup" => "\u{222A}" : false, "dArr" => "\u{21D3}" : false,
    "dagger" => "\u{2020}" : false, "darr" => "\u{2193}" : false,
    "delta" => "\u{3B4}" : false, "diams" => "\u{2666}" : false,
    "empty" => "\u{2205}" : false, "emsp" => "\u{2003}" : false,
    "ensp" => "\u{2002}" : false, "epsilon" => "\u{3B5}" : false,
    "equiv" => "\u{2261}" : false, "eta" => "\u{3B7}" : false,
    "euro" => "\u{20AC}" : false, "exist" => "\u{2203}" : false,
    "fnof" => "\u{192}" : false, "forall" => "\u{2200}" : false,
    "gamma" => "\u{3B3}" : false, "ge" => "\u{2265}" : false,
    "hArr" => "\u{21D4}" : false, "harr" => "\u{2194}" : false,
    "hearts" => "\u{2665}" : false, "hellip" => "\u{2026}" : false,
    "image" => "\u{2111}" : false, "infin" => "\u{221E}" : false,
    "int" => "\u{222B}" : false, "iota" => "\u{3B9}" : false,
    "isin" => "\u{2208}" : false, "kappa" => "\u{3BA}" : false,
    "lArr" => "\u{21D0}" : false, "lambda" => "\u{3BB}" : false,
    "lang" => "\u{27E8}" : false, "larr" => "\u{2190}" : false,
    "lceil" => "\u{2308}" : false, "ldquo" => "\u{201C}" : false,
    "le" => "\u{2264}" : false, "lfloor" => "\u{230A}" : false,
    "lowast" => "\u{2217}" : false, "loz" => "\u{25CA}" : false,
    "lrm" => "\u{200E}" : false, "lsaquo" => "\u{2039}" : false,
    "lsquo" => "\u{2018}" : false, "mdash" => "\u{2014}" : false,
    "minus" => "\u{2212}" : false, "mu" => "\u{3BC}" : false,
    "nabla" => "\u{2207}" : false, "ndash" => "\u{2013}" : false,
    "ne" => "\u{2260}" : false, "ni" => "\u{220B}" : false,
    "notin" => "\u{2209}" : false, "nsub" => "\u{2284}" : false,
    "nu" => "\u{3BD}" : false, "oline" => "\u{203E}" : false,
    "omega" => "\u{3C9}" : false, "omicron" => "\u{3BF}" : false,
    "oplus" => "\u{2295}" : false, "or" => "\u{2228}" : false,
    "otimes" => "\u{2297}" : false, "part" => "\u{2202}" : false,
    "percnt" => "%" : false,     "permil" => "\u{2030}" : false,
    "perp" => "\u{22A5}" : false, "phi" => "\u{3C6}" : false,
    "pi" => "\u{3C0}" : false,   "piv" => "\u{3D6}" : false,
    "prime" => "\u{2032}" : false, "prod" => "\u{220F}" : false,
    "prop" => "\u{221D}" : false, "psi" => "\u{3C8}" : false,
    "rArr" => "\u{21D2}" : false, "radic" => "\u{221A}" : false,
    "rang" => "\u{27E9}" : false, "rarr" => "\u{2192}" : false,
    "rceil" => "\u{2309}" : false, "rdquo" => "\u{201D}" : false,
    "real" => "\u{211C}" : false, "rfloor" => "\u{230B}" : false,
    "rho" => "\u{3C1}" : false, "rlm" => "\u{200F}" : false,
    "rsaquo" => "\u{203A}" : false, "rsquo" => "\u{2019}" : false,
    "sbquo" => "\u{201A}" : false, "scaron" => "\u{161}" : false,
    "sdot" => "\u{22C5}" : false, "sigma" => "\u{3C3}" : false,
    "sigmaf" => "\u{3C2}" : false, "sim" => "\u{223C}" : false,
    "spades" => "\u{2660}" : false, "sub" => "\u{2282}" : false,
    "sube" => "\u{2286}" : false, "sum" => "\u{2211}" : false,
    "sup" => "\u{2283}" : false, "supe" => "\u{2287}" : false,
    "tau" => "\u{3C4}" : false,  "there4" => "\u{2234}" : false,
    "theta" => "\u{3B8}" : false, "thetasym" => "\u{3D1}" : false,
    "thinsp" => "\u{2009}" : false, "tilde" => "\u{2DC}" : false,
    "trade" => "\u{2122}" : false, "uArr" => "\u{21D1}" : false,
    "uarr" => "\u{2191}" : false, "upsih" => "\u{3D2}" : false,
    "upsilon" => "\u{3C5}" : false, "weierp" => "\u{2118}" : false,
    "xi" => "\u{3BE}" : false,   "zeta" => "\u{3B6}" : false,
    "zwj" => "\u{200D}" : false, "zwnj" => "\u{200C}" : false,
];

/// Résultat d'une résolution d'entité : rendu + longueur consommée
/// dans l'entrée (`&` non compris — inclut le `;` s'il est présent).
pub(crate) struct EntityMatch {
    pub value: &'static str,
    pub consumed: usize,
}

/// Résout une référence nommée commençant à `input` (juste après
/// `&`). Matching du plus long préfixe, `;` d'abord puis legacy
/// sans `;` (`in_attribute` applique la règle de l'esperluette
/// ambiguë : jamais devant `=`/alphanumérique).
pub(crate) fn lookup_named(input: &str, in_attribute: bool) -> Option<EntityMatch> {
    // `;` d'abord — matching exact le plus long.
    let mut best: Option<&NamedRef> = None;
    let mut best_len = 0;
    for r in NAMED_REFS {
        if r.name.len() > best_len
            && input.starts_with(r.name)
            && input.as_bytes().get(r.name.len()) == Some(&b';')
        {
            best = Some(r);
            best_len = r.name.len();
        }
    }
    if let Some(r) = best {
        return Some(EntityMatch {
            value: r.value,
            consumed: r.name.len() + 1,
        });
    }
    // Legacy sans `;` — uniquement les entités marquées.
    let after_blocked = |len: usize| -> bool {
        in_attribute
            && input[len..]
                .chars()
                .next()
                .is_some_and(|c| c == '=' || c.is_ascii_alphanumeric())
    };
    let mut best: Option<&NamedRef> = None;
    let mut best_len = 0;
    for r in NAMED_REFS {
        if r.legacy && r.name.len() > best_len && input.starts_with(r.name) {
            // Règle de l'esperluette ambiguë : en attribut, jamais
            // devant `=` ou alphanumérique (`?a=1&amp=x` littéral).
            // Dans le contenu, `&notit` = `¬it` (spec) — pas de blocage.
            if in_attribute && after_blocked(r.name.len()) {
                continue;
            }
            best = Some(r);
            best_len = r.name.len();
        }
    }
    best.map(|r| EntityMatch {
        value: r.value,
        consumed: r.name.len(),
    })
}

/// Remap `windows-1252` pour les références numériques
/// `0x80..=0x9F` (spec : `&#150;` = `–`, pas U+0096).
const NUMERIC_1252: [char; 32] = [
    '€', '\u{81}', '‚', 'ƒ', '„', '…', '†', '‡', 'ˆ', '‰', 'Š', '‹', 'Œ', '\u{8D}', 'Ž', '\u{8F}',
    '\u{90}', '‘', '’', '“', '”', '•', '–', '—', '˜', '™', 'š', '›', 'œ', '\u{9D}', 'ž', 'Ÿ',
];

/// Convertit un codepoint de référence numérique selon la spec :
/// `0x00` → `U+FFFD`, surrogates/sup → `U+FFFD`, `0x80..=0x9F` →
/// remap `windows-1252`.
#[must_use]
pub fn map_numeric(codepoint: u32) -> char {
    match codepoint {
        0x00 => '\u{FFFD}',
        0x80..=0x9F => NUMERIC_1252[(codepoint - 0x80) as usize],
        cp => char::from_u32(cp).unwrap_or('\u{FFFD}'),
    }
}
