// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! CSSOM borné — peuplement de `CSSStyleSheet.cssRules`.
//!
//! Split **textuel** de la feuille en règles top-level (pas de
//! parseur : `explorer-css` est hors périmètre du moteur
//! autonome). Chaque règle devient un objet miroir
//! `{cssText, type}` + `selectorText` (règles de style),
//! `keyText` (keyframes), `conditionText` (`@media`/
//! `@supports`/`@container`), `name` (`@layer`) et `cssRules`
//! imbriqué pour les règles de groupement — la forme lue par
//! les recensements in-page (`sheet.cssRules[i].cssText`).
//!
//! Bornes : profondeur de groupement ≤ 4, ≤ 4096 entrées par
//! liste — une feuille hostile reste un scan linéaire ; les
//! at-rules sans constante héritée (css-cascade-5+ n'en
//! attribue plus) rapportent `type: 0`.

use crate::heap::JsResult;
use crate::value::{ObjId, Value};
use crate::Engine;

/// Profondeur max de groupement (`@media` dans `@supports`…)
/// — au-delà le corps est rapporté en `cssText` plat.
const MAX_DEPTH: usize = 4;
/// Borne d'entrées par liste de règles (feuille hostile).
const MAX_RULES: usize = 4096;

/// Règle textuelle isolée : tranche complète + prélude et corps.
struct RawRule<'a> {
    /// `prelude { body }` ou `prelude ;` — tranche source
    /// complète (sert `cssText`).
    full: &'a str,
    /// Avant `{`/`;`, trimmé.
    prelude: &'a str,
    /// Corps entre accolades (sans les accolades).
    body: Option<&'a str>,
}

/// Saute blancs et commentaires `/*…*/` ; rend la position du
/// premier octet significatif (ou `src.len()`).
fn skip_ws(src: &str, mut i: usize) -> usize {
    let b = src.as_bytes();
    while i < b.len() {
        match b[i] {
            b' ' | b'\t' | b'\n' | b'\r' | 0x0c => i += 1,
            b'/' if b.get(i + 1) == Some(&b'*') => {
                i += 2;
                while i + 1 < b.len() && !(b[i] == b'*' && b[i + 1] == b'/') {
                    i += 1;
                }
                i = (i + 2).min(b.len());
            }
            _ => break,
        }
    }
    i
}

/// Avance jusqu'au prochain `{`, `;` ou fin — commentaires
/// et chaînes `'…'`/`"…"` (échappement `\`) ignorés. Rend
/// `(fin_prelude, delim)`.
fn scan_prelude(src: &str, mut i: usize) -> (usize, u8) {
    let b = src.as_bytes();
    while i < b.len() {
        match b[i] {
            b'{' | b';' => return (i, b[i]),
            b'/' if b.get(i + 1) == Some(&b'*') => {
                i += 2;
                while i + 1 < b.len() && !(b[i] == b'*' && b[i + 1] == b'/') {
                    i += 1;
                }
                i = (i + 2).min(b.len());
            }
            q @ (b'\'' | b'"') => {
                i += 1;
                while i < b.len() && b[i] != q {
                    i += if b[i] == b'\\' { 2 } else { 1 };
                }
                i += 1;
            }
            _ => i += 1,
        }
    }
    (b.len(), 0)
}

/// Position de l'accolade fermante appariée à `{` en `open`
/// — profondeur suivie, commentaires/chaînes ignorés.
fn match_brace(src: &str, open: usize) -> usize {
    let b = src.as_bytes();
    let mut depth = 1usize;
    let mut i = open + 1;
    while i < b.len() && depth > 0 {
        match b[i] {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return i;
                }
            }
            b'/' if b.get(i + 1) == Some(&b'*') => {
                i += 2;
                while i + 1 < b.len() && !(b[i] == b'*' && b[i + 1] == b'/') {
                    i += 1;
                }
                i = (i + 2).min(b.len());
                continue;
            }
            q @ (b'\'' | b'"') => {
                i += 1;
                while i < b.len() && b[i] != q {
                    i += if b[i] == b'\\' { 2 } else { 1 };
                }
            }
            _ => {}
        }
        i += 1;
    }
    b.len()
}

/// Split top-level de `css` en règles — déclarations orphelines
/// (`x: y ;` hors bloc) et texte résiduel ignorés (invalides
/// spec, jamais exposés dans `cssRules`).
fn split_rules(css: &str) -> Vec<RawRule<'_>> {
    let mut out = Vec::new();
    let mut i = 0usize;
    while out.len() < MAX_RULES {
        i = skip_ws(css, i);
        if i >= css.len() {
            break;
        }
        let start = i;
        let (pend, delim) = scan_prelude(css, i);
        if delim == b'{' {
            let close = match_brace(css, pend);
            let full_end = (close + 1).min(css.len());
            out.push(RawRule {
                full: css[start..full_end].trim(),
                prelude: css[start..pend].trim(),
                body: Some(&css[pend + 1..close]),
            });
            i = full_end;
        } else {
            // Statement `prelude ;` (ou fin) : une règle
            // `@x …;` seulement — un non-`@` nu est une
            // déclaration orpheline, ignorée.
            let pre = css[start..pend].trim();
            if pre.starts_with('@') {
                let end = if delim == b';' { pend + 1 } else { pend };
                out.push(RawRule {
                    full: css[start..end].trim(),
                    prelude: pre,
                    body: None,
                });
            }
            i = if delim == b';' { pend + 1 } else { css.len() };
        }
    }
    out
}

/// Constante héritée `CSSRule.type` d'une at-rule (cssom
/// historique — les at-rules post-`@layer` n'en reçoivent
/// plus : `0` honnête).
fn at_rule_type(prelude: &str) -> u32 {
    // `p[1..]` = nom sans `@`, insensible à la casse et
    // préfixé vendeur possible (`@-webkit-keyframes`).
    let p = prelude.as_bytes();
    if p.first() != Some(&b'@') {
        return 1;
    }
    let name = &prelude[1..];
    let end = name
        .find(|c: char| c.is_ascii_whitespace() || c == '{' || c == '(')
        .unwrap_or(name.len());
    let kw = name[..end].to_ascii_lowercase();
    let kw = kw
        .trim_start_matches("-webkit-")
        .trim_start_matches("-moz-")
        .trim_start_matches("-o-");
    match kw {
        "import" => 3,
        "media" => 4,
        "font-face" => 5,
        "page" => 6,
        "keyframes" => 7,
        "namespace" => 10,
        "counter-style" => 11,
        "supports" => 12,
        "font-feature-values" => 14,
        "viewport" => 15,
        "layer" => 16,
        "container" => 18,
        _ => 0,
    }
}

/// Le prélude moins le mot-clé `@x` — `conditionText` de
/// `@media`/`@supports`/`@container`, `name` de `@layer`.
fn at_rule_arg(prelude: &str) -> &str {
    prelude[1..]
        .find(|c: char| c.is_ascii_whitespace() || c == '(')
        .map_or("", |i| prelude[1 + i..].trim())
}

/// `true` pour les at-rules à bloc de **règles** (vs bloc de
/// déclarations : `@font-face`, `@page` hors marges,
/// `@counter-style`…).
fn is_grouping(prelude: &str) -> bool {
    let t = at_rule_type(prelude);
    matches!(t, 4 | 12 | 16 | 18) || {
        let n = prelude[1..].to_ascii_lowercase();
        n.starts_with("starting-style") || n.starts_with("scope")
    }
}

/// Traduit une règle textuelle en objet miroir. `ctx` porte le
/// parent (`page`/`keyframes`) pour typer les enfants.
fn rule_value(e: &mut Engine, r: &RawRule<'_>, depth: usize, parent: u8) -> JsResult<Value> {
    let o = e.new_object();
    let ct = e.str_val(r.full);
    e.set_own(o, "cssText", ct, true);
    let (ty, key_prop, key_val, grouped) = if r.prelude.starts_with('@') {
        let ty = at_rule_type(r.prelude);
        // `@layer a, b ;` (statement) = 17 vs bloc 16.
        let ty = if ty == 16 && r.body.is_none() { 17 } else { ty };
        // Marge `@page` → CSSMarginRule (9) ; at-rule inconnue
        // dans un autre contexte → sa propre constante ou 0.
        let ty = if parent == b'p' && ty == 0 { 9 } else { ty };
        let arg = at_rule_arg(r.prelude);
        let kp = match ty {
            4 | 12 | 18 => "conditionText",
            16 | 17 => "name",
            _ => "",
        };
        let grouped = r.body.is_some() && depth < MAX_DEPTH && is_grouping(r.prelude);
        (ty, kp, arg, grouped)
    } else if parent == b'k' {
        (8, "keyText", r.prelude, false)
    } else {
        (1, "selectorText", r.prelude, false)
    };
    e.set_own(o, "type", Value::Num(f64::from(ty)), true);
    if !key_prop.is_empty() && !key_val.is_empty() {
        let kv = e.str_val(key_val);
        e.set_own(o, key_prop, kv, true);
    }
    if grouped {
        let body = r.body.unwrap_or("");
        let inner: Vec<Value> = split_rules(body)
            .iter()
            .map(|c| rule_value(e, c, depth + 1, b'g'))
            .collect::<JsResult<_>>()?;
        let arr = e.new_array(inner);
        e.set_own(o, "cssRules", arr, true);
    } else if matches!(ty, 6) && r.body.is_some() {
        // `@page` : le corps contient des déclarations ET des
        // règles de marge `@top-left {…}` — les enfants `@x`
        // sont exposés (type 9 via `parent == b'p'`).
        let inner: Vec<Value> = split_rules(r.body.unwrap_or(""))
            .iter()
            .filter(|c| c.prelude.starts_with('@'))
            .map(|c| rule_value(e, c, depth + 1, b'p'))
            .collect::<JsResult<_>>()?;
        if !inner.is_empty() {
            let arr = e.new_array(inner);
            e.set_own(o, "cssRules", arr, true);
        }
    } else if matches!(ty, 7) && r.body.is_some() {
        // `@keyframes` : enfants `from`/`50%`/`to` → type 8.
        let inner: Vec<Value> = split_rules(r.body.unwrap_or(""))
            .iter()
            .filter(|c| !c.prelude.starts_with('@'))
            .map(|c| rule_value(e, c, depth + 1, b'k'))
            .collect::<JsResult<_>>()?;
        let arr = e.new_array(inner);
        e.set_own(o, "cssRules", arr, true);
    }
    // `CSSStyleRule` porte `style` (CSSStyleDeclaration) — non
    // miroité : `cssText` suffit au recensement (borne doc).
    Ok(Value::Obj(o))
}

/// Peuple `cssRules` d'un miroir `CSSStyleSheet` depuis le
/// texte de la feuille — appelé par `document.styleSheets` et
/// `el.sheet`. Texte absent → tableau vide (borné : une
/// feuille non fetchée n'a pas de règles observables).
pub(crate) fn populate_sheet_rules(e: &mut Engine, sheet: ObjId, css: &str) -> JsResult<()> {
    let rules: Vec<Value> = split_rules(css)
        .iter()
        .map(|r| rule_value(e, r, 0, 0))
        .collect::<JsResult<_>>()?;
    if let Some(arr) = crate::misc_apis::sheet_rules_obj(e, Value::Obj(sheet)) {
        if let Some(o) = e.heap.obj_mut(arr) {
            if let crate::heap::ObjKind::Array(elems) = &mut o.kind {
                *elems = rules;
            }
        }
    }
    Ok(())
}
