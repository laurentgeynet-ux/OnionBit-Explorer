// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Tests du parseur et des évaluateurs CSP (CAP-145) :
//! sources, chaînes de repli, cumul de politiques,
//! `form-action`, `base-uri`, inline (nonce/unsafe-inline),
//! `upgrade-insecure-requests`, interdits `<meta>`.

use explorer_net::csp::{
    csp_allows, csp_base_uri_allowed, csp_form_action_allowed, csp_inline_allowed, csp_upgrade,
    CspPolicy, InlineKind,
};
use explorer_net::http::FetchDest;
use explorer_net::url::Url;

fn u(s: &str) -> Url {
    Url::parse(s).unwrap()
}

fn p(header: &str) -> Vec<CspPolicy> {
    vec![CspPolicy::parse(header, false)]
}

const PAGE: &str = "https://h.test/dir/page";

#[test]
fn script_src_self_allows_same_origin_blocks_cross() {
    let csp = p("script-src 'self'");
    let page = u(PAGE);
    assert!(csp_allows(
        &csp,
        &page,
        FetchDest::Script,
        &u("https://h.test/x.js")
    ));
    assert!(!csp_allows(
        &csp,
        &page,
        FetchDest::Script,
        &u("https://evil.test/e.js")
    ));
    // Le port fait partie de l'origine.
    assert!(!csp_allows(
        &csp,
        &page,
        FetchDest::Script,
        &u("https://h.test:8443/x.js")
    ));
}

#[test]
fn default_src_is_fallback_for_all_fetch_dests() {
    let csp = p("default-src 'none'");
    let page = u(PAGE);
    for dest in [
        FetchDest::Script,
        FetchDest::Style,
        FetchDest::Image,
        FetchDest::Font,
        FetchDest::Iframe,
        FetchDest::Empty,
    ] {
        assert!(
            !csp_allows(&csp, &page, dest, &u("https://h.test/x")),
            "{dest:?}"
        );
    }
    // Les navigations top-level ne sont pas des sous-ressources.
    assert!(csp_allows(
        &csp,
        &page,
        FetchDest::Document,
        &u("https://any.test/")
    ));
}

#[test]
fn specific_directive_overrides_default_src() {
    let csp = p("default-src 'none'; img-src https:");
    let page = u(PAGE);
    assert!(csp_allows(
        &csp,
        &page,
        FetchDest::Image,
        &u("https://cdn.test/i.png")
    ));
    assert!(!csp_allows(
        &csp,
        &page,
        FetchDest::Image,
        &u("http://cdn.test/i.png")
    ));
    assert!(!csp_allows(
        &csp,
        &page,
        FetchDest::Script,
        &u("https://h.test/x.js")
    ));
}

#[test]
fn frame_src_falls_back_to_child_src_then_default() {
    let page = u(PAGE);
    let via_child = p("child-src https://cdn.test; default-src 'none'");
    assert!(csp_allows(
        &via_child,
        &page,
        FetchDest::Iframe,
        &u("https://cdn.test/f")
    ));
    let via_default = p("default-src https://cdn.test");
    assert!(csp_allows(
        &via_default,
        &page,
        FetchDest::Iframe,
        &u("https://cdn.test/f")
    ));
    // `frame-src` présente prime sur `child-src`.
    let both = p("frame-src 'none'; child-src https:");
    assert!(!csp_allows(
        &both,
        &page,
        FetchDest::Iframe,
        &u("https://cdn.test/f")
    ));
}

#[test]
fn multiple_policies_restrict_to_intersection() {
    let csp = vec![
        CspPolicy::parse("script-src https:", false),
        CspPolicy::parse("script-src 'self'", false),
    ];
    let page = u(PAGE);
    // La permissive admet https: partout, la restrictive
    // exige 'self' — l'intersection gagne.
    assert!(csp_allows(
        &csp,
        &page,
        FetchDest::Script,
        &u("https://h.test/x.js")
    ));
    assert!(!csp_allows(
        &csp,
        &page,
        FetchDest::Script,
        &u("https://cdn.test/x.js")
    ));
}

#[test]
fn host_source_matching() {
    let page = u(PAGE);
    let csp = p("img-src https://cdn.test https://*.cdn2.test:8443 https://h.test/static/*");
    assert!(csp_allows(
        &csp,
        &page,
        FetchDest::Image,
        &u("https://cdn.test/i")
    ));
    assert!(!csp_allows(
        &csp,
        &page,
        FetchDest::Image,
        &u("http://cdn.test/i")
    ));
    // `*.h` couvre les sous-domaines ET l'apex (spec).
    assert!(csp_allows(
        &csp,
        &page,
        FetchDest::Image,
        &u("https://a.cdn2.test:8443/i")
    ));
    assert!(csp_allows(
        &csp,
        &page,
        FetchDest::Image,
        &u("https://cdn2.test:8443/i")
    ));
    assert!(!csp_allows(
        &csp,
        &page,
        FetchDest::Image,
        &u("https://a.cdn2.test/i")
    ));
    // Chemin exact ou préfixe `*`.
    assert!(csp_allows(
        &csp,
        &page,
        FetchDest::Image,
        &u("https://h.test/static/i.png")
    ));
    assert!(!csp_allows(
        &csp,
        &page,
        FetchDest::Image,
        &u("https://h.test/other/i.png")
    ));
}

/// Issue #21 (vol re-baseline CAP, github : −10 671 px de
/// `doc_h` sur `img-src … images.ctfassets.net/8aevphvgewt8/`)
/// — CSP3 §matching : un path-source terminé par `/` est un
/// **préfixe**, pas une égalité exacte.
#[test]
fn path_source_trailing_slash_is_prefix() {
    let page = u(PAGE);
    let csp = p("img-src https://cdn.test/dir/");
    // `/dir/` lui-même et tout ce qui est dessous.
    assert!(csp_allows(
        &csp,
        &page,
        FetchDest::Image,
        &u("https://cdn.test/dir/")
    ));
    assert!(csp_allows(
        &csp,
        &page,
        FetchDest::Image,
        &u("https://cdn.test/dir/x.webp")
    ));
    assert!(csp_allows(
        &csp,
        &page,
        FetchDest::Image,
        &u("https://cdn.test/dir/a/b/c.webp")
    ));
    // `/dirx` ne partage pas le préfixe `/dir/` — jamais.
    assert!(!csp_allows(
        &csp,
        &page,
        FetchDest::Image,
        &u("https://cdn.test/dirx/y.webp")
    ));
    assert!(!csp_allows(
        &csp,
        &page,
        FetchDest::Image,
        &u("https://cdn.test/other/x.webp")
    ));
    // Sans le `/` terminal : égalité exacte inchangée —
    // `/dir` ne couvre ni `/dir/` ni `/dir/x`.
    let exact = p("img-src https://cdn.test/dir");
    assert!(csp_allows(
        &exact,
        &page,
        FetchDest::Image,
        &u("https://cdn.test/dir")
    ));
    assert!(!csp_allows(
        &exact,
        &page,
        FetchDest::Image,
        &u("https://cdn.test/dir/")
    ));
    assert!(!csp_allows(
        &exact,
        &page,
        FetchDest::Image,
        &u("https://cdn.test/dir/x")
    ));
}

#[test]
fn star_and_scheme_sources() {
    let page = u(PAGE);
    assert!(csp_allows(
        &p("img-src *"),
        &page,
        FetchDest::Image,
        &u("https://w.test/i")
    ));
    assert!(csp_allows(
        &p("img-src data:"),
        &page,
        FetchDest::Image,
        &u("data:image/png;base64,x")
    ));
    assert!(!csp_allows(
        &p("img-src data:"),
        &page,
        FetchDest::Image,
        &u("https://h.test/i")
    ));
    // `*` n'englobe pas `data:`/`blob:` (spec).
    assert!(!csp_allows(
        &p("img-src *"),
        &page,
        FetchDest::Image,
        &u("data:x")
    ));
}

#[test]
fn first_duplicate_directive_wins() {
    let page = u(PAGE);
    let csp = p("script-src 'none'; script-src 'self'");
    assert!(!csp_allows(
        &csp,
        &page,
        FetchDest::Script,
        &u("https://h.test/x.js")
    ));
}

#[test]
fn keywords_never_match_fetch() {
    let page = u(PAGE);
    // `'unsafe-inline'` n'autorise pas une URL externe.
    let csp = p("script-src 'unsafe-inline'");
    assert!(!csp_allows(
        &csp,
        &page,
        FetchDest::Script,
        &u("https://h.test/x.js")
    ));
}

#[test]
fn form_action_no_default_src_fallback() {
    let page = u(PAGE);
    // `form-action` ne replie PAS sur `default-src` (spec).
    let csp = p("default-src 'none'");
    assert!(csp_form_action_allowed(
        &csp,
        &page,
        &u("https://any.test/go")
    ));
    let csp = p("form-action 'self'");
    assert!(csp_form_action_allowed(
        &csp,
        &page,
        &u("https://h.test/go")
    ));
    assert!(!csp_form_action_allowed(
        &csp,
        &page,
        &u("https://evil.test/go")
    ));
}

#[test]
fn base_uri_no_default_src_fallback() {
    let page = u(PAGE);
    let csp = p("default-src 'none'");
    assert!(csp_base_uri_allowed(&csp, &page, &u("https://cdn.test/")));
    let csp = p("base-uri 'self'");
    assert!(csp_base_uri_allowed(&csp, &page, &u("https://h.test/cdn/")));
    assert!(!csp_base_uri_allowed(&csp, &page, &u("https://cdn.test/")));
}

#[test]
fn inline_blocked_without_unsafe_inline() {
    // Une directive `*-src` présente sans `'unsafe-inline'`
    // bloque l'inline — même `'self'` ne lève pas le blocage.
    let csp = p("script-src 'self'");
    assert!(!csp_inline_allowed(&csp, InlineKind::Script, None));
    assert!(!csp_inline_allowed(&csp, InlineKind::Script, Some("n")));
    // Pas de directive → non restreint.
    assert!(csp_inline_allowed(&p(""), InlineKind::Script, None));
    assert!(csp_inline_allowed(
        &p("img-src 'none'"),
        InlineKind::Script,
        None
    ));
}

#[test]
fn inline_unsafe_inline_and_nonce() {
    let unsafe_ = p("script-src 'unsafe-inline'; style-src 'unsafe-inline'");
    assert!(csp_inline_allowed(&unsafe_, InlineKind::Script, None));
    assert!(csp_inline_allowed(&unsafe_, InlineKind::Style, None));

    let nonced = p("script-src 'nonce-abc'");
    assert!(csp_inline_allowed(&nonced, InlineKind::Script, Some("abc")));
    assert!(!csp_inline_allowed(
        &nonced,
        InlineKind::Script,
        Some("zzz")
    ));
    assert!(!csp_inline_allowed(&nonced, InlineKind::Script, None));
    // `nonce`/`hash` présents → `'unsafe-inline'` ignoré pour
    // script (spec) mais pas pour style.
    let mixed = p("script-src 'nonce-abc' 'unsafe-inline'; style-src 'sha256-x' 'unsafe-inline'");
    assert!(!csp_inline_allowed(&mixed, InlineKind::Script, None));
    assert!(csp_inline_allowed(&mixed, InlineKind::Style, None));
    // `sha*` non vérifiable (pas de SHA-2) → non-matching.
    assert!(!csp_inline_allowed(
        &p("script-src 'sha256-x'"),
        InlineKind::Script,
        None
    ));
}

#[test]
fn inline_all_policies_must_pass() {
    let csp = vec![
        CspPolicy::parse("script-src 'unsafe-inline'", false),
        CspPolicy::parse("script-src 'self'", false),
    ];
    assert!(!csp_inline_allowed(&csp, InlineKind::Script, None));
}

#[test]
fn upgrade_insecure_requests_rewrites() {
    let csp = p("upgrade-insecure-requests");
    assert_eq!(
        csp_upgrade(&csp, &u("http://h.test/a?x=1")).map(|v| v.as_str()),
        Some("https://h.test/a?x=1".to_string())
    );
    assert_eq!(
        csp_upgrade(&csp, &u("ws://h.test/")).map(|v| v.as_str()),
        Some("wss://h.test/".to_string())
    );
    assert!(csp_upgrade(&csp, &u("https://h.test/")).is_none());
    assert!(csp_upgrade(&csp, &u("data:x")).is_none());
    // Sans la directive → pas de réécriture.
    assert!(csp_upgrade(&p("default-src 'self'"), &u("http://h.test/")).is_none());
}

#[test]
fn meta_forbidden_directives_are_dropped() {
    let pol = CspPolicy::parse(
        "frame-ancestors 'none'; sandbox; report-uri /r; img-src 'none'",
        true,
    );
    let page = u(PAGE);
    // Les directives interdites par méta ont disparu ;
    // `img-src` survit.
    assert!(!csp_allows(
        &[pol],
        &page,
        FetchDest::Image,
        &u("https://h.test/i")
    ));
}

#[test]
fn empty_policy_restricts_nothing() {
    let csp: Vec<CspPolicy> = Vec::new();
    let page = u(PAGE);
    assert!(csp_allows(
        &csp,
        &page,
        FetchDest::Script,
        &u("https://any.test/x")
    ));
    assert!(csp_inline_allowed(&csp, InlineKind::Script, None));
    assert!(csp_form_action_allowed(
        &csp,
        &page,
        &u("https://any.test/")
    ));
}

#[test]
fn malformed_headers_degrade_closed_per_directive() {
    let page = u(PAGE);
    // `script-src` vide (aucune source) = `'none'` effectif :
    // la directive existe mais rien ne matche → refus.
    let csp = p("script-src");
    assert!(!csp_allows(
        &csp,
        &page,
        FetchDest::Script,
        &u("https://h.test/x")
    ));
    // Source invalide → non-matching, les autres restent.
    let csp = p("script-src 'self' http://[bad");
    assert!(csp_allows(
        &csp,
        &page,
        FetchDest::Script,
        &u("https://h.test/x")
    ));
}
