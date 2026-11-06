// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! `Atom` — nom interné process-wide (style `AtomicString` de
//! Blink / `AtomString` de V8) pour `tag_name` et
//! `Attribute.name` (AR-13).
//!
//! Un `Atom` **est** son texte : `&'static str` copiable,
//! `as_str()` gratuit, aucun verrou ni indirection à la
//! lecture. L'interning déduplique par `thread_local` (le DOM
//! est single-writer — zéro synchronisation) : un nom unique
//! n'est alloué qu'une fois, vie du processus — le même
//! compromis que les vrais navigateurs (la table d'atomes n'est
//! pas comptée dans `Dom::heap_bytes`, process-globale).
//!
//! Correctness sans dédup : `PartialEq` = égalité **textuelle**
//! (fast-path pointeur quand la dédup a frappé) — deux atomes
//! du même nom restent égaux même si deux threads les ont
//! internés séparément ; `Hash` et `Ord` suivent le texte pour
//! rester cohérents avec `Eq`.

use std::borrow::Borrow;
use std::cell::RefCell;
use std::cmp::Ordering;
use std::collections::HashSet;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::ops::Deref;

/// Nom interné — `&'static str` dédupliquée.
///
/// `Copy` (16 o). Comparaisons et méthodes `str` via `Deref` :
/// `atom == "div"`, `atom.len()`, `atom.starts_with("x")`,
/// `atom.eq_ignore_ascii_case(s)`, `format!("{atom}")` —
/// la migration depuis `String` est quasi transparente.
#[derive(Clone, Copy)]
pub struct Atom(&'static str);

/// Noms pré-semés dans l'interneur de chaque thread — la même
/// `&'static str` partout : le fast-path pointeur de `PartialEq`
/// frappe aussi cross-thread pour le vocabulaire courant.
const SEED: &[&str] = &[
    // Balises HTML courantes.
    "a",
    "address",
    "area",
    "article",
    "aside",
    "audio",
    "b",
    "base",
    "bdi",
    "bdo",
    "blockquote",
    "body",
    "br",
    "button",
    "canvas",
    "caption",
    "cite",
    "code",
    "col",
    "colgroup",
    "data",
    "datalist",
    "dd",
    "del",
    "details",
    "dfn",
    "dialog",
    "div",
    "dl",
    "dt",
    "em",
    "embed",
    "fieldset",
    "figcaption",
    "figure",
    "footer",
    "form",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "head",
    "header",
    "hgroup",
    "hr",
    "html",
    "i",
    "iframe",
    "img",
    "input",
    "ins",
    "kbd",
    "label",
    "legend",
    "li",
    "link",
    "main",
    "map",
    "mark",
    "menu",
    "meta",
    "meter",
    "nav",
    "noscript",
    "object",
    "ol",
    "optgroup",
    "option",
    "output",
    "p",
    "picture",
    "pre",
    "progress",
    "q",
    "rp",
    "rt",
    "ruby",
    "s",
    "samp",
    "script",
    "search",
    "section",
    "select",
    "slot",
    "small",
    "source",
    "span",
    "strong",
    "style",
    "sub",
    "summary",
    "sup",
    "table",
    "tbody",
    "td",
    "template",
    "textarea",
    "tfoot",
    "th",
    "thead",
    "time",
    "title",
    "tr",
    "track",
    "u",
    "ul",
    "var",
    "video",
    "wbr",
    // Balises étrangères (case ajustée par le tree-builder).
    "svg",
    "path",
    "rect",
    "circle",
    "ellipse",
    "line",
    "polyline",
    "polygon",
    "g",
    "defs",
    "use",
    "symbol",
    "text",
    "tspan",
    "image",
    "foreignObject",
    "desc",
    "linearGradient",
    "radialGradient",
    "stop",
    "clipPath",
    "mask",
    "pattern",
    "marker",
    "view",
    "animate",
    "animateTransform",
    "animateMotion",
    "set",
    "filter",
    "feGaussianBlur",
    "feColorMatrix",
    "feBlend",
    "feComposite",
    "feFlood",
    "feMerge",
    "feMorphology",
    "feOffset",
    "mi",
    "mo",
    "mn",
    "ms",
    "mtext",
    "math",
    "annotation-xml",
    // Attributs courants.
    "id",
    "class",
    "style",
    "href",
    "src",
    "alt",
    "title",
    "type",
    "name",
    "value",
    "rel",
    "target",
    "width",
    "height",
    "role",
    "lang",
    "dir",
    "hidden",
    "disabled",
    "checked",
    "selected",
    "readonly",
    "required",
    "placeholder",
    "maxlength",
    "minlength",
    "min",
    "max",
    "step",
    "pattern",
    "multiple",
    "autofocus",
    "autocomplete",
    "action",
    "method",
    "enctype",
    "for",
    "form",
    "formaction",
    "formmethod",
    "formtarget",
    "formnovalidate",
    "formenctype",
    "srcset",
    "sizes",
    "loading",
    "decoding",
    "fetchpriority",
    "crossorigin",
    "referrerpolicy",
    "integrity",
    "async",
    "defer",
    "nomodule",
    "charset",
    "content",
    "media",
    "nonce",
    "xmlns",
    "viewBox",
    "d",
    "fill",
    "stroke",
    "points",
    "cx",
    "cy",
    "r",
    "rx",
    "ry",
    "x",
    "y",
    "x1",
    "x2",
    "y1",
    "y2",
    "transform",
    "preserveAspectRatio",
    "xlink:href",
    "data",
    "poster",
    "preload",
    "controls",
    "autoplay",
    "loop",
    "muted",
    "playsinline",
    "kind",
    "srclang",
    "label",
    "default",
    "sandbox",
    "allow",
    "allowfullscreen",
    "srcdoc",
    "tabindex",
    "accesskey",
    "contenteditable",
    "draggable",
    "spellcheck",
    "translate",
    "aria-label",
    "aria-hidden",
    "aria-expanded",
    "aria-controls",
    "aria-describedby",
    "aria-labelledby",
    "aria-live",
    "aria-pressed",
    "aria-selected",
    "onclick",
    "onload",
    "onerror",
    "onsubmit",
    "oninput",
    "onchange",
    "onkeydown",
    "onkeyup",
    "onkeypress",
    "onmousedown",
    "onmouseup",
    "onmousemove",
    "onmouseover",
    "onmouseout",
    "itemprop",
    "itemscope",
    "itemtype",
    "download",
    "ping",
    "hreflang",
    "coords",
    "shape",
    "usemap",
    "ismap",
    "open",
    "colspan",
    "rowspan",
    "headers",
    "scope",
    "abbr",
    "start",
    "reversed",
    "wrap",
    "rows",
    "cols",
    "list",
    "accept",
    "accept-charset",
    "size",
    "cellpadding",
    "cellspacing",
    "border",
    "align",
    "valign",
    "bgcolor",
    "color",
    "face",
    "datetime",
    "pubdate",
];

thread_local! {
    static INTERNER: RefCell<HashSet<&'static str>> = RefCell::new(
        SEED.iter().copied().collect()
    );
}

fn intern(name: &str) -> Atom {
    INTERNER.with(|set| {
        let mut set = set.borrow_mut();
        if let Some(&s) = set.get(name) {
            return Atom(s);
        }
        // Vie du processus — le compromis AtomicString : un nom
        // unique n'est jamais libéré ni ré-alloué.
        let leaked: &'static str = Box::leak(name.to_owned().into_boxed_str());
        set.insert(leaked);
        Atom(leaked)
    })
}

impl Atom {
    /// Interne `name` — retourne l'atome dédupliqué.
    #[must_use]
    pub fn new(name: &str) -> Self {
        intern(name)
    }

    /// Texte résolu — gratuit (l'atome *est* le texte).
    #[must_use]
    pub fn as_str(self) -> &'static str {
        self.0
    }
}

impl fmt::Debug for Atom {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl fmt::Display for Atom {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl Deref for Atom {
    type Target = str;
    fn deref(&self) -> &str {
        self.0
    }
}

impl AsRef<str> for Atom {
    fn as_ref(&self) -> &str {
        self.0
    }
}

impl Borrow<str> for Atom {
    fn borrow(&self) -> &str {
        self.0
    }
}

impl From<&str> for Atom {
    fn from(s: &str) -> Self {
        intern(s)
    }
}

impl From<&String> for Atom {
    fn from(s: &String) -> Self {
        intern(s)
    }
}

impl From<String> for Atom {
    fn from(s: String) -> Self {
        intern(&s)
    }
}

impl PartialEq for Atom {
    /// Fast-path pointeur (dédup frappée — courant grâce au seed)
    /// puis égalité textuelle : deux atomes du même nom sont
    /// égaux même internés sur des threads différents.
    fn eq(&self, other: &Self) -> bool {
        std::ptr::eq(self.0, other.0) || self.0 == other.0
    }
}

impl Eq for Atom {}

impl PartialEq<str> for Atom {
    fn eq(&self, other: &str) -> bool {
        self.0 == other
    }
}

impl PartialEq<&str> for Atom {
    fn eq(&self, other: &&str) -> bool {
        self.0 == *other
    }
}

impl PartialEq<String> for Atom {
    fn eq(&self, other: &String) -> bool {
        self.0 == other.as_str()
    }
}

impl PartialEq<&String> for Atom {
    fn eq(&self, other: &&String) -> bool {
        self.0 == other.as_str()
    }
}

impl PartialOrd for Atom {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Atom {
    /// Ordre **textuel** — cohérent avec `Eq` même quand la
    /// dédup n'a pas frappé (deux textes égaux, deux atomes).
    fn cmp(&self, other: &Self) -> Ordering {
        self.0.cmp(other.0)
    }
}

impl PartialOrd<str> for Atom {
    fn partial_cmp(&self, other: &str) -> Option<Ordering> {
        Some(self.0.cmp(other))
    }
}

impl Hash for Atom {
    /// Hash **textuel** — obligatoire pour rester cohérent avec
    /// l'`Eq` textuel (deux atomes de même texte hashent égal).
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.0.hash(state);
    }
}
