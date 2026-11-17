// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

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

/// `src` dans boa — issue canonisée (budgets bornés comme jscore :
/// boucles et pile, les valeurs par défaut boa bornent déjà la
/// récursion à 512 et la pile VM à 10 K).
#[must_use]
pub fn eval_boa(src: &str) -> Outcome {
    let mut ctx = Context::default();
    ctx.runtime_limits_mut().set_loop_iteration_limit(2_000_000);
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
