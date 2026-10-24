// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Évaluateur borné de media queries pour `matchMedia` (CAP-161).
//!
//! Le vrai évaluateur vit dans `explorer-css` (`media_matches`/
//! `StyleEnv`) — crate PIPELINE, hors de portée de jscore (autonome).
//! Ici : sous-ensemble volontairement borné des queries réelles —
//! `,` (ou), `not`/`only` préfixés, type `screen`/`all` vs autres
//! (`print`/`speech` → faux), conditions `(feature[: value])`
//! jointes par `and` ou `or` :
//!
//! - `width`/`height` (+ `min-`/`max-`) en `px`/`em`/`rem` contre
//!   le viewport de session ;
//! - `aspect-ratio`/`min-`/`max-` (`a/b`) ;
//! - `orientation: landscape|portrait` ;
//! - `prefers-color-scheme`, `prefers-reduced-motion`,
//!   `prefers-reduced-transparency`, `prefers-contrast`,
//!   `forced-colors`, `display-mode` ;
//! - `hover`/`any-hover`, `pointer`/`any-pointer` (desktop :
//!   `hover`/`fine` d'env, `none`/`coarse` niés) ;
//! - `color`/`min-color` (8 bits), `color-gamut` (`srgb` seul),
//!   `resolution`/`min-resolution`/`max-resolution` (1 dppx),
//!   `grid` (0), `update` (`fast`), `scripting` (`enabled` —
//!   ce moteur exécute du JS), `monochrome` (0) ;
//! - feature nue `(feature)` = « non zéro » (spec) ;
//! - feature inconnue → clause fausse (dégradation spec :
//!   query invalide ≙ `not all`).
//!
//! Bornes : pas d'évaluation `style()`/`supports()`, pas de
//! `change` events (l'env est un snapshot posé au spawn — voir
//! [`crate::dom_host::BrowserHooks::media_env`]), plages
//! `(400px < width < 800px)` non supportées (opérateurs de
//! plage Media Queries Level 4 — clause fausse).

/// Environnement média de la session (schéma, entrées) —
/// poussé dans le moteur par `JsEngine::set_media_env` au
/// `spawn_engine`. Le viewport n'est **pas** ici : les
/// requêtes `width`/`height`/`orientation` lisent
/// `BrowserHooks::viewport` en direct (même source que
/// `innerWidth`/`screen.*`, swap inclus). Snapshot : les
/// `MediaQueryList` relisent l'env courant à la demande,
/// jamais de `change` dispatché.
#[derive(Debug, Clone, Copy, Default)]
#[allow(clippy::struct_excessive_bools)]
pub struct MediaEnv {
    /// `prefers-color-scheme: dark` de la session.
    pub dark: bool,
    /// `prefers-reduced-motion: reduce` de la session.
    pub reduced_motion: bool,
    /// Pointeur capable de survol (`hover: hover`).
    pub hover: bool,
    /// Pointeur précis (`pointer: fine` — souris desktop).
    pub pointer_fine: bool,
}

/// `matchMedia(q).matches` — évalue `q` contre `env` +
/// `vp` (viewport `(w,h)` px CSS — `hooks.viewport`).
/// Query invalide/inconnue → `false` (spec : `not all`).
pub(crate) fn eval_media_query(q: &str, env: MediaEnv, vp: (u32, u32)) -> bool {
    // `,` = ou logique — une clause vraie suffit.
    q.split(',').any(|part| eval_query(part.trim(), env, vp))
}

/// Une clause `not|only? type? (and cond)*` — `not` inverse le
/// résultat complet (spec : négation de la clause entière).
fn eval_query(s: &str, env: MediaEnv, vp: (u32, u32)) -> bool {
    let mut s = s.trim().to_ascii_lowercase();
    let neg = s.starts_with("not ");
    if neg {
        s = s[4..].trim().to_string();
    }
    if let Some(rest) = s.strip_prefix("only ") {
        s = rest.trim().to_string();
    }
    // Type de média : ident en tête jusqu'à `(` ou ` and `.
    let head = s.split(['(', ' ']).next().unwrap_or_default();
    let (ty, rest) = if head.is_empty() {
        (String::new(), s.clone())
    } else {
        (head.to_string(), s[head.len()..].trim_start().to_string())
    };
    let ty_ok = match ty.as_str() {
        "" | "all" | "screen" => true,
        // Types connus inapplicables + identifiants inconnus
        // (spec : type inconnu = query toujours fausse).
        _ => false,
    };
    let mut ok = ty_ok;
    if ty_ok && !rest.is_empty() {
        ok = eval_joined(&rest, env, vp);
    }
    if neg {
        !ok
    } else {
        ok
    }
}

/// Jointure des conditions parenthésées après le type :
/// `(a) and (b) …` ou `(a) or (b) …`. Mélanger `and` et `or`
/// au même niveau est invalide (MQ4 impose alors des parens
/// imbriqués — non supporté) → clause fausse. Tout jeton
/// non-parenthésé rend la clause fausse.
fn eval_joined(rest: &str, env: MediaEnv, vp: (u32, u32)) -> bool {
    let mut r = rest.trim_start();
    let mut conds: Vec<(bool, &str)> = Vec::new(); // (is_or, cond)
    let mut join_or = false;
    loop {
        if let Some(x) = r.strip_prefix("and ") {
            join_or = false;
            r = x.trim_start();
        } else if let Some(x) = r.strip_prefix("or ") {
            join_or = true;
            r = x.trim_start();
        }
        if r.is_empty() {
            break;
        }
        let Some(start) = r.find('(') else {
            return false;
        };
        if !r[..start].trim().is_empty() {
            return false;
        }
        let Some(end) = r[start..].find(')') else {
            return false;
        };
        conds.push((join_or, &r[start + 1..start + end]));
        r = r[start + end + 1..].trim_start();
    }
    if conds.is_empty() {
        return true;
    }
    // Les jointures vivent entre opérandes — conds[1..].
    let joins = &conds[1..];
    if joins.iter().any(|(o, _)| *o) && joins.iter().any(|(o, _)| !*o) {
        return false;
    }
    if joins.iter().any(|(o, _)| *o) {
        conds.iter().any(|(_, c)| eval_cond(c, env, vp))
    } else {
        conds.iter().all(|(_, c)| eval_cond(c, env, vp))
    }
}

/// Une condition `(feature[: value])` — ` or ` interne supporté
/// (`(a or b)` est légal dans une parenthèse, Media Q. Level 4).
fn eval_cond(inner: &str, env: MediaEnv, vp: (u32, u32)) -> bool {
    inner
        .split(" or ")
        .any(|part| eval_atom(part.trim(), env, vp))
}

/// Atome `feature[: value]` — préfixes `min-`/`max-` deviennent
/// des comparaisons `>=`/`<=` (spec).
fn eval_atom(atom: &str, env: MediaEnv, vp: (u32, u32)) -> bool {
    let (feature, value) = match atom.split_once(':') {
        Some((f, v)) => (f.trim(), Some(v.trim())),
        None => (atom.trim(), None),
    };
    let (op, feat) = if let Some(f) = feature.strip_prefix("min-") {
        (Op::Min, f)
    } else if let Some(f) = feature.strip_prefix("max-") {
        (Op::Max, f)
    } else {
        (Op::Eq, feature)
    };
    let Some(v) = value else {
        // Feature nue — « non zéro » (spec `if the media
        // feature would be true in a boolean context`).
        return bare_truthy(feat, env, vp);
    };
    match feat {
        "width" => cmp_px(vp.0, v, op),
        "height" => cmp_px(vp.1, v, op),
        "aspect-ratio" => cmp_ratio(vp, v, op),
        "orientation" => {
            op == Op::Eq
                && match v {
                    "landscape" => vp.0 >= vp.1,
                    "portrait" => vp.1 > vp.0,
                    _ => false,
                }
        }
        "prefers-color-scheme" => op == Op::Eq && (v == "dark") == env.dark,
        "prefers-reduced-motion" => op == Op::Eq && (v == "reduce") == env.reduced_motion,
        "prefers-reduced-transparency" | "prefers-contrast" | "contrast" => {
            op == Op::Eq && matches!(v, "no-preference" | "less")
        }
        "forced-colors" => op == Op::Eq && v == "none",
        "display-mode" => op == Op::Eq && v == "browser",
        "hover" | "any-hover" => op == Op::Eq && (v == "hover") == env.hover,
        "pointer" | "any-pointer" => {
            op == Op::Eq
                && match v {
                    "fine" => env.pointer_fine,
                    "coarse" => !env.pointer_fine,
                    _ => false,
                }
        }
        "color" => cmp_num(8, v, op),
        "color-gamut" => op == Op::Eq && v == "srgb",
        "resolution" => cmp_f64(1.0, v.trim_end_matches("dppx"), op),
        "grid" | "monochrome" => cmp_num(0, v, op),
        "update" => op == Op::Eq && v == "fast",
        "scripting" => op == Op::Eq && v == "enabled",
        _ => false,
    }
}

/// Feature **nue** `(f)` — vraie si « non zéro » dans l'env.
fn bare_truthy(feat: &str, env: MediaEnv, vp: (u32, u32)) -> bool {
    match feat {
        "width" => vp.0 > 0,
        "height" => vp.1 > 0,
        "color" | "color-gamut" | "update" | "scripting" => true,
        "hover" | "any-hover" => env.hover,
        "pointer" | "any-pointer" => env.pointer_fine,
        _ => false,
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Op {
    Eq,
    Min,
    Max,
}

/// `(min-|max-)?width: Npx|Nem|Nrem` — comparaison px.
fn cmp_px(actual: u32, v: &str, op: Op) -> bool {
    let Some(want) = px_len(v) else { return false };
    cmp_ord(f64::from(actual), want, op)
}

/// `a/b` — ratio `w/h` (spec : `a` ou `a/b`, `a` ≙ `a/1`).
fn cmp_ratio(vp: (u32, u32), v: &str, op: Op) -> bool {
    let (a, b) = match v.split_once('/') {
        Some((a, b)) => (a.trim().parse::<f64>(), b.trim().parse::<f64>()),
        None => (v.trim().parse::<f64>(), Ok(1.0)),
    };
    let (Ok(a), Ok(b)) = (a, b) else { return false };
    if b == 0.0 || vp.1 == 0 {
        return false;
    }
    cmp_ord(f64::from(vp.0) / f64::from(vp.1), a / b, op)
}

/// Entier simple (`min-color`, `grid`, `monochrome`).
fn cmp_num(actual: u32, v: &str, op: Op) -> bool {
    let Ok(want) = v.parse::<f64>() else {
        return false;
    };
    cmp_ord(f64::from(actual), want, op)
}

/// Flottant (`resolution` en dppx).
fn cmp_f64(actual: f64, v: &str, op: Op) -> bool {
    let Ok(want) = v.parse::<f64>() else {
        return false;
    };
    cmp_ord(actual, want, op)
}

/// `Npx` / `Nem` / `Nrem` (em/rem = 16 px — base navigateur).
fn px_len(v: &str) -> Option<f64> {
    let v = v.trim();
    let (num, mul) = if let Some(n) = v.strip_suffix("px") {
        (n, 1.0)
    } else if let Some(n) = v.strip_suffix("rem") {
        (n, 16.0)
    } else if let Some(n) = v.strip_suffix("em") {
        (n, 16.0)
    } else {
        (v, 1.0)
    };
    num.trim().parse::<f64>().ok().map(|n| n * mul)
}

fn cmp_ord(actual: f64, want: f64, op: Op) -> bool {
    match op {
        Op::Min => actual >= want,
        Op::Max => actual <= want,
        Op::Eq => (actual - want).abs() < f64::EPSILON,
    }
}
