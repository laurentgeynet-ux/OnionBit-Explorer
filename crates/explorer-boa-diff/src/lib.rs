// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0
#![forbid(unsafe_code)]

//! Oracle différentiel `jscore ↔ boa` — même script dans les deux
//! moteurs, issues canonisées puis comparées. Outil de conformité
//! (piste audit `docs/audit-jscore-vs-boa.md`) — **jamais** de boa
//! dans le code livré.
//!
//! Canonisation : la valeur de complétion passe par `ToString`
//! spec des deux côtés (`describe` côté jscore, `to_string` côté
//! boa) ; les erreurs sont ramenées à `NomDeClasse: message` ;
//! un échec interne (limite runtime, panic isolé) devient
//! `Internal` — une divergence `Internal` vs `Value` est un signal
//! fort (crash/budget) pas un faux négatif.

use boa_engine::{Context, JsError, JsNativeErrorKind, Source};

/// Issue canonique d'une évaluation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// `ToString` de la valeur de complétion.
    Value(String),
    /// Exception levée : `Nom: message`.
    Throw(String),
    /// Erreur de syntaxe (parse).
    Syntax(String),
    /// Échec interne au moteur (limite, deadline, panic isolé).
    Internal(String),
}

impl Outcome {
    /// Classe d'erreur `Name` extraite de `Throw("Name: msg")`.
    #[must_use]
    pub fn thrown_name(&self) -> Option<String> {
        match self {
            Self::Throw(s) => Some(s.split(':').next().unwrap_or(s.as_str()).trim().to_string()),
            _ => None,
        }
    }
}

/// `src` dans `explorer-jscore` — issue canonisée.
#[must_use]
pub fn eval_jscore(src: &str) -> Outcome {
    let mut e = explorer_jscore::Engine::new();
    match e.eval(src) {
        Ok(v) => Outcome::Value(e.describe(v)),
        Err(explorer_jscore::Error::Syntax { message, .. }) => Outcome::Syntax(message),
        Err(explorer_jscore::Error::Uncaught(s)) => Outcome::Throw(s),
        Err(explorer_jscore::Error::Deadline) => Outcome::Internal("deadline".to_string()),
    }
}

/// APIs **hôte** que jscore installe (navigateur) et que boa ne
/// fournit pas — le prelude donne à boa des implémentations de
/// référence spec-following pour la surface exercée par les
/// sondes, afin que ces APIs restent testables en différentiel
/// (le shim sert d'oracle quand boa n'en est pas un).
/// Évalué dans chaque contexte boa avant la sonde ; limité au
/// strict nécessaire — tout élargissement de la surface probe
/// demande l'extension du shim correspondant.
/// **Code volontairement plat** : le parseur récursif de boa
/// 0.22-dev déborde sa pile native sur les `else if` chaînés
/// imbriqués — `if` séquentiels et déclarations top-level.
const BOA_PRELUDE: &str = r"
function __diffClone(x, seen) {
    if (typeof x === 'function') throw new TypeError('function could not be cloned');
    if (x === null || typeof x !== 'object') return x;
    if (seen.has(x)) return seen.get(x);
    if (x instanceof Map) {
        const m = new Map(); seen.set(x, m);
        for (const [k, e] of x) m.set(__diffClone(k, seen), __diffClone(e, seen));
        return m;
    }
    if (x instanceof Set) {
        const s = new Set(); seen.set(x, s);
        for (const e of x) s.add(__diffClone(e, seen));
        return s;
    }
    if (Array.isArray(x)) {
        const a = new Array(x.length); seen.set(x, a);
        for (let i = 0; i < x.length; i++) a[i] = __diffClone(x[i], seen);
        return a;
    }
    const o = {}; seen.set(x, o);
    for (const k of Object.keys(x)) o[k] = __diffClone(x[k], seen);
    return o;
}
function structuredClone(v) { return __diffClone(v, new Map()); }
globalThis.Intl = {
    Collator: class Collator {
        constructor(loc, opts) { this.loc = loc; }
        compare(a, b) { const x = String(a), y = String(b); return x < y ? -1 : x > y ? 1 : 0; }
    },
    NumberFormat: class NumberFormat {
        constructor(loc, opts) { this.loc = loc; }
        format(n) {
            const neg = Object.is(n, -0) || n < 0;
            const s = String(Math.abs(n));
            const dot = s.indexOf('.');
            let i = dot < 0 ? s : s.slice(0, dot);
            const f = dot < 0 ? '' : s.slice(dot);
            let out = '';
            while (i.length > 3) { out = ',' + i.slice(-3) + out; i = i.slice(0, -3); }
            return (neg ? '-' : '') + i + out + f;
        }
    },
};
Error.captureStackTrace = function captureStackTrace(target) {
    target.stack = '';
};
";

/// `src` dans boa — issue canonisée (budgets bornés comme jscore :
/// boucles et pile, les valeurs par défaut boa bornent déjà la
/// récursion à 512 et la pile VM à 10 K). Le [`BOA_PRELUDE`] est
/// évalué d'abord : il installe les APIs hôte de référence.
#[must_use]
pub fn eval_boa(src: &str) -> Outcome {
    let mut ctx = Context::default();
    ctx.runtime_limits_mut().set_loop_iteration_limit(2_000_000);
    if let Err(err) = ctx.eval(Source::from_bytes(BOA_PRELUDE)) {
        return Outcome::Internal(format!("prelude: {err}"));
    }
    match ctx.eval(Source::from_bytes(src)) {
        Ok(v) => match v.to_string(&mut ctx) {
            Ok(s) => Outcome::Value(s.to_std_string_escaped()),
            Err(err) => boa_err(&mut ctx, &err),
        },
        Err(err) => boa_err(&mut ctx, &err),
    }
}

/// `JsError` → canon : erreur native → `Nom: message` (kind +
/// message spec) ; erreur opaque (valeur JS levée non-Error) →
/// `Throw` de son `ToString` ; panic/limite → `Internal`.
fn boa_err(ctx: &mut Context, err: &JsError) -> Outcome {
    match err.try_native(ctx) {
        Ok(ne) => {
            let name = match ne.kind() {
                JsNativeErrorKind::Type => "TypeError",
                JsNativeErrorKind::Range => "RangeError",
                JsNativeErrorKind::Reference => "ReferenceError",
                JsNativeErrorKind::Syntax => "SyntaxError",
                JsNativeErrorKind::Eval => "EvalError",
                JsNativeErrorKind::Uri => "URIError",
                JsNativeErrorKind::Aggregate(_) => "AggregateError",
                _ => "Error",
            };
            if name == "SyntaxError" {
                Outcome::Syntax(ne.message().to_string())
            } else {
                Outcome::Throw(format!("{name}: {}", ne.message()))
            }
        }
        // Erreur opaque = valeur JS levée (`throw {}`, `throw "s"`)
        // ou panic/limite — l'un et l'autre non natifs.
        Err(t) => Outcome::Internal(t.to_string()),
    }
}

/// Deux issues **concordent** si : même variante, valeurs
/// `ToString` identiques, et `Throw` même classe (le message diffère
/// légalement entre moteurs — seule la classe est spec).
#[must_use]
pub fn concordant(a: &Outcome, b: &Outcome) -> bool {
    match (a, b) {
        (Outcome::Syntax(_), Outcome::Syntax(_)) => true,
        (Outcome::Throw(_), Outcome::Throw(_)) => a.thrown_name() == b.thrown_name(),
        (Outcome::Value(x), Outcome::Value(y)) | (Outcome::Internal(x), Outcome::Internal(y)) => {
            x == y
        }
        _ => false,
    }
}
