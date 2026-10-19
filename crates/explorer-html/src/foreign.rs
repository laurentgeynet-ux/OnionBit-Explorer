// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Contenu étranger SVG/MathML (HTML §13.2.6.5, R-15) :
//! tables d'ajustement de la spec + classification des points
//! d'intégration et de la liste de « breakout ».

use explorer_dom::{ElementData, Namespace};

use crate::token::TokenAttribute;

/// Noms de balises SVG dont la casse est ajustée (spec « adjust
/// SVG tag names » — le tokenizer a tout mis en minuscules).
const SVG_TAGS: &[(&str, &str)] = &[
    ("altglyph", "altGlyph"),
    ("altglyphdef", "altGlyphDef"),
    ("altglyphitem", "altGlyphItem"),
    ("animatecolor", "animateColor"),
    ("animatemotion", "animateMotion"),
    ("animatetransform", "animateTransform"),
    ("clippath", "clipPath"),
    ("feblend", "feBlend"),
    ("fecolormatrix", "feColorMatrix"),
    ("fecomponenttransfer", "feComponentTransfer"),
    ("fecomposite", "feComposite"),
    ("feconvolvematrix", "feConvolveMatrix"),
    ("fediffuselighting", "feDiffuseLighting"),
    ("fedisplacementmap", "feDisplacementMap"),
    ("fedistantlight", "feDistantLight"),
    ("fedropshadow", "feDropShadow"),
    ("feflood", "feFlood"),
    ("fefunca", "feFuncA"),
    ("fefuncb", "feFuncB"),
    ("fefuncg", "feFuncG"),
    ("fefuncr", "feFuncR"),
    ("fegaussianblur", "feGaussianBlur"),
    ("feimage", "feImage"),
    ("femerge", "feMerge"),
    ("femergenode", "feMergeNode"),
    ("femorphology", "feMorphology"),
    ("feoffset", "feOffset"),
    ("fepointlight", "fePointLight"),
    ("fespecularlighting", "feSpecularLighting"),
    ("fespotlight", "feSpotLight"),
    ("fetile", "feTile"),
    ("feturbulence", "feTurbulence"),
    ("foreignobject", "foreignObject"),
    ("glyphref", "glyphRef"),
    ("lineargradient", "linearGradient"),
    ("radialgradient", "radialGradient"),
    ("textpath", "textPath"),
];

/// Attributs SVG dont la casse est ajustée (spec « adjust SVG
/// attributes »).
const SVG_ATTRS: &[(&str, &str)] = &[
    ("attributename", "attributeName"),
    ("attributetype", "attributeType"),
    ("basefrequency", "baseFrequency"),
    ("baseprofile", "baseProfile"),
    ("calcmode", "calcMode"),
    ("clippathunits", "clipPathUnits"),
    ("diffuseconstant", "diffuseConstant"),
    ("edgemode", "edgeMode"),
    ("filterunits", "filterUnits"),
    ("glyphref", "glyphRef"),
    ("gradienttransform", "gradientTransform"),
    ("gradientunits", "gradientUnits"),
    ("kernelmatrix", "kernelMatrix"),
    ("kernelunitlength", "kernelUnitLength"),
    ("keypoints", "keyPoints"),
    ("keysplines", "keySplines"),
    ("keytimes", "keyTimes"),
    ("lengthadjust", "lengthAdjust"),
    ("limitingconeangle", "limitingConeAngle"),
    ("markerheight", "markerHeight"),
    ("markerunits", "markerUnits"),
    ("markerwidth", "markerWidth"),
    ("maskcontentunits", "maskContentUnits"),
    ("maskunits", "maskUnits"),
    ("numoctaves", "numOctaves"),
    ("pathlength", "pathLength"),
    ("patterncontentunits", "patternContentUnits"),
    ("patterntransform", "patternTransform"),
    ("patternunits", "patternUnits"),
    ("pointsatx", "pointsAtX"),
    ("pointsaty", "pointsAtY"),
    ("pointsatz", "pointsAtZ"),
    ("preservealpha", "preserveAlpha"),
    ("preserveaspectratio", "preserveAspectRatio"),
    ("primitiveunits", "primitiveUnits"),
    ("refx", "refX"),
    ("refy", "refY"),
    ("repeatcount", "repeatCount"),
    ("repeatdur", "repeatDur"),
    ("requiredextensions", "requiredExtensions"),
    ("requiredfeatures", "requiredFeatures"),
    ("specularconstant", "specularConstant"),
    ("specularexponent", "specularExponent"),
    ("spreadmethod", "spreadMethod"),
    ("startoffset", "startOffset"),
    ("stddeviation", "stdDeviation"),
    ("stitchtiles", "stitchTiles"),
    ("surfacescale", "surfaceScale"),
    ("systemlanguage", "systemLanguage"),
    ("tablevalues", "tableValues"),
    ("targetx", "targetX"),
    ("targety", "targetY"),
    ("textlength", "textLength"),
    ("viewbox", "viewBox"),
    ("viewtarget", "viewTarget"),
    ("xchannelselector", "xChannelSelector"),
    ("ychannelselector", "yChannelSelector"),
    ("zoomandpan", "zoomAndPan"),
];

/// Balises qui font sortir du contenu étranger (spec §13.2.6.5
/// « breakout » — `font` seulement avec `color`/`face`/`size`).
const BREAKOUT: &[&str] = &[
    "b",
    "big",
    "blockquote",
    "body",
    "br",
    "center",
    "code",
    "dd",
    "div",
    "dl",
    "dt",
    "em",
    "embed",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "head",
    "hr",
    "i",
    "img",
    "li",
    "listing",
    "menu",
    "meta",
    "nobr",
    "ol",
    "p",
    "pre",
    "ruby",
    "s",
    "small",
    "span",
    "strong",
    "strike",
    "sub",
    "sup",
    "table",
    "tt",
    "u",
    "ul",
    "var",
];

/// Nom de balise ajusté en casse pour le namespace donné
/// (`foreignobject` → `foreignObject` en SVG seulement).
pub(crate) fn adjust_tag(name: &str, ns: Namespace) -> &str {
    if ns != Namespace::Svg {
        return name;
    }
    SVG_TAGS
        .binary_search_by(|(k, _)| k.cmp(&name))
        .ok()
        .map_or(name, |i| SVG_TAGS[i].1)
}

/// Nom d'attribut ajusté en casse (SVG camelCase + `definitionURL`
/// `MathML`). Les `xlink:*`/`xml:*`/`xmlns*` arrivent déjà sous leur
/// forme préfixée — le dump les attend tels quels.
pub(crate) fn adjust_attr(name: &str, ns: Namespace) -> &str {
    match ns {
        Namespace::Svg => SVG_ATTRS
            .binary_search_by(|(k, _)| k.cmp(&name))
            .ok()
            .map_or(name, |i| SVG_ATTRS[i].1),
        Namespace::MathMl => {
            if name == "definitionurl" {
                "definitionURL"
            } else {
                name
            }
        }
        Namespace::Html => name,
    }
}

/// `true` si la balise ouvrante fait sortir du contenu étranger
/// (spec §13.2.6.5 — `font` uniquement porteur d'un des trois
/// attributs présentationnels).
pub(crate) fn is_breakout(name: &str, attrs: &[TokenAttribute]) -> bool {
    if BREAKOUT.contains(&name) {
        return true;
    }
    name == "font"
        && attrs
            .iter()
            .any(|a| matches!(a.name.as_str(), "color" | "face" | "size"))
}

/// Point d'intégration `MathML` (`mi|mo|mn|ms|mtext`) — les enfants
/// non-`mglyph`/`malignmark` y sont traités en HTML.
pub(crate) fn is_mathml_text_ip(el: &ElementData) -> bool {
    el.namespace == Namespace::MathMl
        && matches!(el.tag_name.as_str(), "mi" | "mo" | "mn" | "ms" | "mtext")
}

/// `true` si l'élément étranger est un terminateur de scope /
/// catégorie « special » (spec : `MathML` mi/mo/mn/ms/mtext +
/// annotation-xml, SVG foreignObject/desc/title).
pub(crate) fn is_foreign_scope_boundary(el: &ElementData) -> bool {
    match el.namespace {
        Namespace::MathMl => matches!(
            el.tag_name.as_str(),
            "mi" | "mo" | "mn" | "ms" | "mtext" | "annotation-xml"
        ),
        Namespace::Svg => matches!(el.tag_name.as_str(), "foreignObject" | "desc" | "title"),
        Namespace::Html => false,
    }
}

/// Point d'intégration HTML (SVG `foreignObject|desc|title` +
/// `MathML` `annotation-xml` à `encoding` html/xhtml) — les start
/// tags et le texte y suivent les règles HTML normales.
pub(crate) fn is_html_ip(el: &ElementData) -> bool {
    match el.namespace {
        Namespace::Svg => matches!(el.tag_name.as_str(), "foreignObject" | "desc" | "title"),
        Namespace::MathMl => {
            el.tag_name == "annotation-xml"
                && el.attributes.iter().any(|a| {
                    a.name == "encoding"
                        && (a.value.eq_ignore_ascii_case("text/html")
                            || a.value.eq_ignore_ascii_case("application/xhtml+xml"))
                })
        }
        Namespace::Html => false,
    }
}
