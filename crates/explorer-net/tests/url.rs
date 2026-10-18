// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Tests du parseur/résolveur d'URL — périmètre WHATWG étape 3.

use explorer_net::url::{Origin, Url, UrlError};

fn u(s: &str) -> Url {
    Url::parse(s).unwrap_or_else(|e| panic!("{s} -> {e}"))
}

fn res(base: &str, rel: &str) -> Url {
    Url::resolve(&u(base), rel).unwrap_or_else(|e| panic!("{base} + {rel} -> {e}"))
}

// -- parsing absolu -----------------------------------------------------

#[test]
fn parse_absolute_basic() {
    let url = u("https://example.com/path?q=1#f");
    assert_eq!(url.scheme(), "https");
    assert_eq!(url.host(), "example.com");
    assert_eq!(url.path(), "/path");
    assert_eq!(url.query(), Some("q=1"));
    assert_eq!(url.fragment(), Some("f"));
}

#[test]
fn scheme_and_host_lowercased() {
    let url = u("HTTP://EXAMPLE.COM/A");
    assert_eq!(url.scheme(), "http");
    assert_eq!(url.host(), "example.com");
    // Le path n'est PAS cassé en minuscules.
    assert_eq!(url.path(), "/A");
}

#[test]
fn default_port_elided() {
    assert_eq!(u("http://example.com:80/a").port(), None);
    assert_eq!(u("https://example.com:443/a").port(), None);
    assert_eq!(u("http://example.com:8080/a").port(), Some(8080));
    assert_eq!(
        u("https://example.com:8080/a").as_str(),
        "https://example.com:8080/a"
    );
}

#[test]
fn effective_port() {
    assert_eq!(u("http://a/b").effective_port(), Some(80));
    assert_eq!(u("https://a/b").effective_port(), Some(443));
    assert_eq!(u("http://a:99/b").effective_port(), Some(99));
}

#[test]
fn empty_path_becomes_slash() {
    assert_eq!(u("http://a").path(), "/");
    assert_eq!(u("http://a?x").path(), "/");
}

#[test]
fn userinfo_parsed() {
    let url = u("http://user:pass@example.com/");
    assert_eq!(url.as_str(), "http://user:pass@example.com/");
}

#[test]
fn dot_segments_resolved() {
    assert_eq!(u("http://a/b/c/../d").path(), "/b/d");
    assert_eq!(u("http://a/b/./d").path(), "/b/d");
    assert_eq!(u("http://a/../x").path(), "/x");
    assert_eq!(u("http://a/b/..").path(), "/");
    assert_eq!(u("http://a/b/.").path(), "/b/");
}

#[test]
fn percent_encoding() {
    // Espace → %20, %XX valides préservés en majuscules.
    assert_eq!(u("http://a/a b").path(), "/a%20b");
    assert_eq!(u("http://a/%2f").path(), "/%2F");
    // UTF-8 → séquence encodée.
    let url = u("http://a/héllo");
    assert_eq!(url.path(), "/h%C3%A9llo");
}

#[test]
fn ipv4_forms() {
    assert_eq!(u("http://127.0.0.1/").host(), "127.0.0.1");
    // Formes réduites WHATWG.
    assert_eq!(u("http://127.1/").host(), "127.0.0.1");
    assert_eq!(u("http://0x7f.1/").host(), "127.0.0.1");
    assert_eq!(u("http://0177.0.0.1/").host(), "127.0.0.1");
    // `256.0.0.1` n'est pas une IPv4 valide → WHATWG le traite
    // comme un *domaine* (reg-name), pas une erreur.
    assert_eq!(u("http://256.0.0.1/").host(), "256.0.0.1");
}

#[test]
fn ipv6_brackets() {
    let url = u("http://[::1]:8080/a");
    assert_eq!(url.host(), "[::1]");
    assert_eq!(url.port(), Some(8080));
}

#[test]
fn idna_international_domain() {
    // bücher.de → punycode.
    let url = u("http://bücher.de/");
    assert!(url.host().starts_with("xn--"), "host = {}", url.host());
    assert_eq!(url.host(), "xn--bcher-kva.de");
}

#[test]
fn opaque_schemes_verbatim() {
    let url = u("data:text/html,<b>hi</b>");
    assert_eq!(url.scheme(), "data");
    // Le chemin opaque reste verbatim (pas de normalisation ./..).
    assert!(url.path().contains("text/html"));

    let about = u("about:blank");
    assert_eq!(about.scheme(), "about");
    assert_eq!(about.path(), "blank");
}

#[test]
fn file_url() {
    let url = u("file:///C:/docs/a.txt");
    assert_eq!(url.scheme(), "file");
    assert_eq!(url.host(), "");
    assert_eq!(url.path(), "/C:/docs/a.txt");
}

#[test]
fn query_and_fragment() {
    let url = u("http://a/p?x=1&y=2#sec");
    assert_eq!(url.query(), Some("x=1&y=2"));
    assert_eq!(url.fragment(), Some("sec"));
    assert_eq!(url.path_query(), "/p?x=1&y=2");
}

// -- résolution relative (RFC 3986 §5.2) --------------------------------

#[test]
fn resolve_absolute() {
    assert_eq!(res("http://a/b", "https://c/d").as_str(), "https://c/d");
}

#[test]
fn resolve_network_path() {
    assert_eq!(
        res("https://a/b", "//cdn.example/x").as_str(),
        "https://cdn.example/x"
    );
}

#[test]
fn resolve_absolute_path() {
    assert_eq!(res("http://a/b/c", "/d").as_str(), "http://a/d");
}

#[test]
fn resolve_relative_path() {
    assert_eq!(res("http://a/b/c", "d").as_str(), "http://a/b/d");
    assert_eq!(res("http://a/b/c", "d/e").as_str(), "http://a/b/d/e");
    assert_eq!(res("http://a/b/", "d").as_str(), "http://a/b/d");
}

#[test]
fn resolve_dotdot() {
    assert_eq!(res("http://a/b/c/d", "../e").as_str(), "http://a/b/e");
    assert_eq!(res("http://a/b/c/d", "../../e").as_str(), "http://a/e");
    assert_eq!(res("http://a/", "../x").as_str(), "http://a/x");
}

#[test]
fn resolve_query_and_fragment() {
    assert_eq!(res("http://a/b?q=1", "?q=2").as_str(), "http://a/b?q=2");
    assert_eq!(res("http://a/b", "#f").as_str(), "http://a/b#f");
    assert_eq!(res("http://a/b?x#old", "#new").as_str(), "http://a/b?x#new");
    // Référence vide → base sans fragment.
    assert_eq!(res("http://a/b#f", "").as_str(), "http://a/b");
}

#[test]
fn resolve_relative_no_base_errors() {
    assert!(matches!(
        Url::parse("relative/path"),
        Err(UrlError::RelativeWithoutBase(_))
    ));
    assert!(matches!(
        Url::parse(""),
        Err(UrlError::RelativeWithoutBase(_))
    ));
}

#[test]
fn whitespace_and_tabs_ignored() {
    assert_eq!(u("  http://a/b  ").as_str(), "http://a/b");
    assert_eq!(u("http://a/\tb").as_str(), "http://a/b");
}

// -- origine --------------------------------------------------------------

#[test]
fn origin_tuple() {
    assert_eq!(
        u("https://example.com/a").origin(),
        Origin::Tuple {
            scheme: "https".into(),
            host: "example.com".into(),
            port: 443,
        }
    );
    assert_eq!(
        u("http://example.com:8080/a").origin(),
        Origin::Tuple {
            scheme: "http".into(),
            host: "example.com".into(),
            port: 8080,
        }
    );
}

#[test]
fn origin_opaque() {
    assert_eq!(u("data:text/html,x").origin(), Origin::Opaque);
    assert_eq!(u("about:blank").origin(), Origin::Opaque);
    assert_eq!(u("file:///a").origin(), Origin::Opaque);
}

// -- erreurs ---------------------------------------------------------------

#[test]
fn invalid_port() {
    assert!(matches!(
        Url::parse("http://a:abc/"),
        Err(UrlError::InvalidPort(_))
    ));
    assert!(matches!(
        Url::parse("http://a:99999/"),
        Err(UrlError::InvalidPort(_))
    ));
}

#[test]
fn empty_host_rejected() {
    assert!(Url::parse("http:///path").is_err());
    assert!(Url::parse("http://").is_err());
}

#[test]
fn roundtrip_serialization() {
    for s in [
        "https://example.com/a?b=c#d",
        "http://u:p@h:8080/x",
        "data:text/plain,hello",
        "about:blank",
    ] {
        assert_eq!(u(s).as_str(), s, "roundtrip {s}");
    }
}
