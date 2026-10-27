// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Content-Security-Policy (CAP-145) — enforcement des
//! directives fetch + inline de la réponse top-level.
//!
//! La politique vit dans `policy.rs` : ce module porte le
//! parseur d'en-tête `Content-Security-Policy:` (toutes les
//! valeurs, chacune doit autoriser — plusieurs CSP se
//! cumulent par restriction) et les évaluateurs.
//!
//! Périmètre livré (fetch-directives §6.7) :
//! `default-src`, `script-src`, `style-src`, `img-src`,
//! `font-src`, `connect-src`, `media-src`, `frame-src`
//! (repli `child-src` → `default-src`), `form-action`
//! (sans repli `default-src`, spec), `upgrade-insecure-
//! requests`. Sources : `'none'`, `*`, `'self'`,
//! scheme-sources (`https:`, `data:`…), host-sources
//! `[scheme://][*.]host[:port][/path]`, keywords
//! `'unsafe-*'`/`nonce-*`/`sha*-*` (non-URL → jamais en
//! fetch, utilisés par [`csp_inline_allowed`]).
//!
//! Bornes assumées :
//! - `Content-Security-Policy-Report-Only` est **ignoré**
//!   (aucune violation n'est rapportée — pas de cible
//!   `report-uri`/`report-to` implémentée) ;
//! - directives `report-uri`, `report-to`, `require-
//!   trusted-types-for`, `trusted-types`, `sandbox`,
//!   `navigate-to` (retirée de la spec) ignorées ;
//! - sources hash `sha256/384/512-…` non vérifiables sans
//!   SHA-2 dans le workspace → **non-matching** (strict) ;
//! - `script-src-elem`/`-attr` et `style-src-elem`/`-attr`
//!   ignorés — granularité au seul `*-src` ;
//! - `'strict-dynamic'` non implémenté — les host-sources
//!   restent exigées au lieu d'être relâchées pour les
//!   scripts approuvés (fail-closed : plus restrictif que
//!   la spec, jamais plus permissif) ;
//! - `frame-ancestors` reste évalué dans `policy::
//!   frame_embedding_allowed` (réponse du sous-contexte) ;
//! - `<meta http-equiv=Content-Security-Policy>` est
//!   admise : directives `frame-ancestors`/`sandbox`/
//!   `report-uri` y sont ignorées par spec — le parseur
//!   (`via_meta`) les écarte.

use crate::http::FetchDest;
use crate::url::Url;

/// Une politique CSP parsée — directives en ordre, noms
/// minuscules. `via_meta` = provenance `<meta http-equiv>`
/// (les directives interdites par méta ont été écartées
/// au parse).
#[derive(Debug, Clone)]
pub struct CspPolicy {
    /// `(nom, sources)` en ordre de déclaration — le
    /// **premier** duplicata gagne (spec §serialized :
    /// les suivants sont ignorés).
    directives: Vec<(String, Vec<String>)>,
}

/// Sorte d'item inline évalué (`'unsafe-inline'`/`nonce`) —
/// script ou style : le bypass `nonce`/`hash` ignore
/// `'unsafe-inline'` **uniquement** pour script (spec
/// §strict-dynamic).
#[derive(Debug, Clone, Copy)]
pub enum InlineKind {
    /// `<script>` inline / `eval`-adjacent.
    Script,
    /// `<style>` inline (les attributs `style=` restent
    /// non gatés — borne documentée).
    Style,
}

impl CspPolicy {
    /// Parse un en-tête `Content-Security-Policy:` — `;`
    /// sépare les directives, espaces les tokens ; noms
    /// normalisés minuscules ; premier duplicata gagne.
    /// `via_meta` écarte `frame-ancestors`/`sandbox`/
    /// `report-uri`/`report-to` (spec §meta : ignorées en
    /// méta).
    #[must_use]
    pub fn parse(header: &str, via_meta: bool) -> Self {
        const META_FORBIDDEN: [&str; 4] = ["frame-ancestors", "sandbox", "report-uri", "report-to"];
        let mut directives: Vec<(String, Vec<String>)> = Vec::new();
        for d in header.split(';') {
            let mut tok = d.split_whitespace();
            let Some(name) = tok.next() else { continue };
            let name = name.to_ascii_lowercase();
            if via_meta && META_FORBIDDEN.contains(&name.as_str()) {
                continue;
            }
            if directives.iter().any(|(n, _)| n == &name) {
                continue; // duplicata ignoré (spec)
            }
            directives.push((name, tok.map(str::to_string).collect()));
        }
        Self { directives }
    }

    /// Sources d'une directive avec chaîne de repli —
    /// `fallbacks` essaie dans l'ordre (`frame-src` →
    /// `child-src` → `default-src`). `None` = aucune
    /// directive applicable → **non restreint**.
    fn sources<'a>(&'a self, chain: &[&str]) -> Option<&'a [String]> {
        for name in chain {
            if let Some((_, s)) = self.directives.iter().find(|(n, _)| n == name) {
                return Some(s);
            }
        }
        None
    }

    /// `upgrade-insecure-requests` présent.
    fn has_uir(&self) -> bool {
        self.directives
            .iter()
            .any(|(n, _)| n == "upgrade-insecure-requests")
    }
}

/// Chaîne de repli d'un `FetchDest` (CSP §fetch) —
/// `Document` n'est pas une sous-ressource : toujours
/// admis (`frame-ancestors` gouverne l'embarquement,
/// `navigate-to` n'existe plus).
fn dest_chain(dest: FetchDest) -> Option<&'static [&'static str]> {
    match dest {
        FetchDest::Document => None,
        FetchDest::Script => Some(&["script-src", "default-src"]),
        FetchDest::Style => Some(&["style-src", "default-src"]),
        FetchDest::Image => Some(&["img-src", "default-src"]),
        FetchDest::Font => Some(&["font-src", "default-src"]),
        FetchDest::Iframe => Some(&["frame-src", "child-src", "default-src"]),
        FetchDest::Empty => Some(&["connect-src", "default-src"]),
    }
}

/// Fetch `target` depuis `page` admis par **toutes** les
/// politiques (`&&` sur chaque CSP — la plus restrictive
/// réduit). Directive absente → non restreint ; présente
/// → au moins une source matche (`'none'` seul = refus).
#[must_use]
pub fn csp_allows(policies: &[CspPolicy], page: &Url, dest: FetchDest, target: &Url) -> bool {
    let Some(chain) = dest_chain(dest) else {
        return true;
    };
    policies.iter().all(|p| {
        p.sources(chain)
            .is_none_or(|srcs| srcs.iter().any(|s| source_matches(s, page, target)))
    })
}

/// `<form action>` admis par `form-action` (spec : pas de
/// repli `default-src`) — chaîne propre.
#[must_use]
pub fn csp_form_action_allowed(policies: &[CspPolicy], page: &Url, target: &Url) -> bool {
    policies.iter().all(|p| {
        p.sources(&["form-action"])
            .is_none_or(|srcs| srcs.iter().any(|s| source_matches(s, page, target)))
    })
}

/// `<base href>` admis par `base-uri` (spec : pas de repli
/// `default-src`) — une base bloquée est ignorée (le
/// document garde sa propre URL pour base), jamais fatale.
#[must_use]
pub fn csp_base_uri_allowed(policies: &[CspPolicy], page: &Url, target: &Url) -> bool {
    policies.iter().all(|p| {
        p.sources(&["base-uri"])
            .is_none_or(|srcs| srcs.iter().any(|s| source_matches(s, page, target)))
    })
}

/// Item inline (`<script>`/`<style>`) admis : par
/// **chaque** politique, une source lève le blocage —
/// `'unsafe-inline'` (sauf `script-src` avec source
/// `nonce`/`hash` : ignorée, spec §strict) ou
/// `nonce-…` égalant l'attribut `nonce=` de l'élément.
/// `sha*-…` non vérifiables sans SHA-2 → non-matching.
/// L'origine ne gouverne pas l'inline — pas de `page`.
#[must_use]
pub fn csp_inline_allowed(policies: &[CspPolicy], kind: InlineKind, nonce: Option<&str>) -> bool {
    let chain: &[&str] = match kind {
        InlineKind::Script => &["script-src", "default-src"],
        InlineKind::Style => &["style-src", "default-src"],
    };
    policies.iter().all(|p| {
        let Some(srcs) = p.sources(chain) else {
            return true;
        };
        let has_nonce_or_hash = srcs
            .iter()
            .any(|s| s.starts_with("'nonce-") || s.starts_with("'sha"));
        let unsafe_inline = srcs.iter().any(|s| s == "'unsafe-inline'")
            && !(matches!(kind, InlineKind::Script) && has_nonce_or_hash);
        let nonce_hit = nonce.is_some_and(|n| {
            !n.is_empty() && srcs.iter().any(|s| s.as_str() == format!("'nonce-{n}'"))
        });
        unsafe_inline || nonce_hit
    })
}

/// `upgrade-insecure-requests` : la politique réécrit les
/// cibles non-sûres — `http:` → `https:`, `ws:` → `wss:`
/// (port implicite réinitialisé avec le schéma).
#[must_use]
pub fn csp_upgrade(policies: &[CspPolicy], target: &Url) -> Option<Url> {
    if !policies.iter().any(CspPolicy::has_uir) {
        return None;
    }
    let to = match target.scheme() {
        "http" => "https",
        "ws" => "wss",
        _ => return None,
    };
    let mut s = String::with_capacity(target.as_str().len() + 1);
    s.push_str(to);
    s.push_str(&target.as_str()[target.scheme().len()..]);
    Url::parse(&s).ok()
}

/// `src` (source-expression) matche-t-elle `target` sous
/// `page` (CSP §matching) : `'self'` = origine de la page ;
/// `*` = http(s)/ws(s) ; scheme-source `xxx:` ; host-source
/// `[scheme://][*.]host[:port][/chemin]` — `*` nu en position
/// hôte = tout hôte (chemin en suffixe honoré, §matching
/// « path-part ») ; keywords/`nonce`/`hash`/`'none'`/vide →
/// jamais en fetch.
fn source_matches(src: &str, page: &Url, target: &Url) -> bool {
    let s = src.trim();
    if s == "*" {
        return matches!(target.scheme(), "http" | "https" | "ws" | "wss");
    }
    if s == "'self'" {
        return target.origin() == page.origin();
    }
    if s.is_empty() || s.starts_with('\'') {
        return false; // keywords — pas des URL
    }
    // Scheme-source : `https:`/`data:`/`wss:`… (pas de
    // `//`, alphabet scheme RFC3986).
    if let Some(scheme) = s.strip_suffix(':') {
        if !scheme.is_empty()
            && scheme
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
        {
            return target.scheme() == scheme.to_ascii_lowercase();
        }
    }
    // Host-source : `[scheme://][*.]host[:port][/path]` —
    // le chemin garde son `/` de tête : `t.path()` inclut
    // le séparateur (`h.test/static` = path `/static`).
    let low = s.to_ascii_lowercase();
    let (scheme_pat, rest) = match low.split_once("://") {
        Some((sch, rest)) => (Some(sch), rest),
        None => (None, low.as_str()),
    };
    let (host_port, path_pat) = match rest.split_once('/') {
        Some((hp, p)) => (hp, Some(format!("/{p}"))),
        None => (rest, None),
    };
    let (host_pat, port_pat) = match host_port.split_once(':') {
        Some((h, p)) => (h, Some(p)),
        None => (host_port, None),
    };
    let th = target.host();
    // `*` nu en position hôte = tout hôte (spec §matching :
    // `https://*` admet n'importe quel domaine https — vol 32
    // dropbox `style-src https://*` refusait chaque feuille) ;
    // distinct de `*.suffixe` (sous-domaines + apex).
    let host_ok = if host_pat == "*" {
        true
    } else if let Some(suffix) = host_pat.strip_prefix("*.") {
        th == suffix || th.ends_with(&format!(".{suffix}"))
    } else {
        th == host_pat
    };
    if !host_ok {
        return false;
    }
    if let Some(sp) = scheme_pat {
        if target.scheme() != sp {
            return false;
        }
    }
    if let Some(pp) = port_pat {
        if pp != "*" {
            let Ok(n) = pp.parse::<u16>() else {
                return false;
            };
            // Port effectif de la cible : explicite ou défaut
            // du schéma (spec §matching).
            let tport = target.port().unwrap_or(match target.scheme() {
                "http" | "ws" => 80,
                "https" | "wss" => 443,
                _ => 0,
            });
            if tport != n {
                return false;
            }
        }
    }
    if let Some(path) = path_pat {
        let p = target.path();
        // §matching : comparaison ASCII insensible à la
        // casse. Un chemin terminé par `/` matche en
        // **préfixe** (`host/dir/` ⇒ `/dir/*` — issue #21) ;
        // suffixe `*` = préfixe aussi ; sinon égalité exacte.
        // `get(..len)` — jamais de slice hors frontière UTF-8.
        let ok = if path.ends_with('/') {
            p.get(..path.len())
                .is_some_and(|head| head.eq_ignore_ascii_case(&path))
        } else if let Some(prefix) = path.strip_suffix('*') {
            p.get(..prefix.len())
                .is_some_and(|head| head.eq_ignore_ascii_case(prefix))
        } else {
            p.eq_ignore_ascii_case(&path)
        };
        if !ok {
            return false;
        }
    }
    true
}
