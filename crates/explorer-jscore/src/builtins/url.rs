// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Builtins JS — section extraite de `builtins.rs` (JS-C40, pur
//! reformatage mécanique : sémantique et tests inchangés).

#[allow(clippy::wildcard_imports)]
use super::*;

// ------------------------------------------------------------------
// JS-C16 — reduceRight, locale-fallbacks, URL/URLSearchParams
// ------------------------------------------------------------------

pub(crate) fn array_reduce_right(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let (id, cb, _this_arg) = iter_cb(e, this.clone(), args, "reduceRight")?;
    // Itération vivante inversée (`iter_get`, JS-C335) : la cb
    // voit le tableau peuplé — le `take_elems` d'avant le vidait
    // pendant la boucle *et* le laissait vide si la cb jetait
    // (le `put_elems` suivait le `?`, §B-90).
    let len = iter_len(e, id)? as i64;
    let (mut acc, start) = if let Some(v) = args.get(1) {
        (v.clone(), len - 1)
    } else {
        // Sans init, acc = dernier élément *présent* (spec).
        let mut i = len - 1;
        let mut found = None;
        while i >= 0 {
            if let Some(v) = iter_get(e, id, i as usize)? {
                found = Some(v);
                break;
            }
            i -= 1;
        }
        let Some(v) = found else {
            return Err(e.type_error("reduceRight of empty array"));
        };
        (v, i - 1)
    };
    let mut i = start;
    while i >= 0 {
        if let Some(v) = iter_get(e, id, i as usize)? {
            acc = e.call_value(
                cb.clone(),
                Value::Undefined,
                &[acc, v, Value::Num(i as f64), this.clone()],
            )?;
        }
        i -= 1;
    }
    Ok(acc)
}

/// `localeCompare` (déviation documentée : ordre par point de
/// code Unicode, pas de collation CLDR) — `-1/0/1`.
pub(crate) fn string_locale_compare(
    e: &mut Engine,
    this: Value,
    args: &[Value],
) -> JsResult<Value> {
    let s = this_str(e, this)?;
    let o = e.to_js_string(arg(args, 0))?;
    let r = match s.cmp(&o) {
        std::cmp::Ordering::Less => -1.0,
        std::cmp::Ordering::Equal => 0.0,
        std::cmp::Ordering::Greater => 1.0,
    };
    Ok(Value::Num(r))
}

/// `Number.prototype.toLocaleString` — repli `toString` (pas de
/// locale, déviation documentée).
pub(crate) fn number_to_locale(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    number_to_string(e, this, args)
}

// -- URL (WHATWG subset) ---------------------------------------------

/// Parts décomposées d'une URL — pas d'IDNA ni de validation
/// d'hôte stricte (subset navigateur : schéma/authority/path/
/// query/fragment + résolution relative basique).
pub(crate) struct UrlParts {
    scheme: String,
    user: String,
    pass: String,
    host: String,
    port: String,
    path: String,
    query: String,
    frag: String,
}

const URL_DEFAULT_PORTS: &[(&str, &str)] = &[
    ("http", "80"),
    ("https", "443"),
    ("ws", "80"),
    ("wss", "443"),
    ("ftp", "21"),
];

pub(crate) fn url_has_origin(scheme: &str) -> bool {
    matches!(scheme, "http" | "https" | "ws" | "wss" | "ftp" | "file")
}

/// `p.scheme` — accesseur (champs privés ; `WebSocket` JS-C73
/// valide `ws`/`wss`).
pub(crate) fn url_scheme(p: &UrlParts) -> &str {
    &p.scheme
}

/// `p.origin` — `scheme://host[:port]` ou `"null"` pour les
/// origines opaques (`about:`, données…) — sert aux URL objets
/// `blob:<origin>/<id>` (JS-C70).
pub(crate) fn url_origin(p: &UrlParts) -> String {
    if !url_has_origin(&p.scheme) || p.host.is_empty() {
        return "null".to_string();
    }
    if p.port.is_empty() {
        format!("{}://{}", p.scheme, p.host)
    } else {
        format!("{}://{}:{}", p.scheme, p.host, p.port)
    }
}

/// Découpe `user:pass@host:port` (authority après `//`).
pub(crate) fn url_split_authority(auth: &str, p: &mut UrlParts) {
    let (up, hp) = match auth.rsplit_once('@') {
        Some((u, h)) => (u, h),
        None => ("", auth),
    };
    if let Some((u, pw)) = up.split_once(':') {
        p.user = u.to_string();
        p.pass = pw.to_string();
    } else {
        p.user = up.to_string();
    }
    // Port = `:digits` en fin d'authority (host nu IPv6 `[…]` ok).
    if let Some(brk) = hp.rsplit_once(':') {
        if !brk.1.is_empty() && brk.1.chars().all(|c| c.is_ascii_digit()) && !brk.0.ends_with(']') {
            let default = URL_DEFAULT_PORTS
                .iter()
                .find(|(s, _)| s == &p.scheme.as_str())
                .map_or("", |(_, d)| *d);
            p.host = url_host_norm(brk.0, &p.scheme);
            p.port = if brk.1 == default {
                String::new()
            } else {
                brk.1.to_string()
            };
            return;
        }
    }
    p.host = url_host_norm(hp, &p.scheme);
}

/// Normalisation d'hôte (§B-116) : IDNA/UTS-46 + lowercase
/// pour les schémas spéciaux (`idna` workspace, déjà consommé
/// par `explorer-net`) ; IPv6 `[…]` et schémas non-hiérarchiques
/// passent bruts. Échec `domain_to_ascii` = hôte gardé brut
/// (tolérant — la spec rejetterait l'URL entière).
fn url_host_norm(host: &str, scheme: &str) -> String {
    if host.is_empty() || host.starts_with('[') || !url_has_origin(scheme) {
        return host.to_string();
    }
    match idna::domain_to_ascii(host) {
        Ok(a) if !a.is_empty() => a,
        _ => host.to_ascii_lowercase(),
    }
}

/// Normalise les segments `.`/`..` d'un chemin.
pub(crate) fn url_norm_path(path: &str) -> String {
    let mut segs: Vec<&str> = Vec::new();
    for seg in path.split('/') {
        match seg {
            "." => {}
            ".." => {
                segs.pop();
            }
            _ => segs.push(seg),
        }
    }
    let out = segs.join("/");
    let mut out = if path.starts_with('/') && !out.starts_with('/') {
        format!("/{out}")
    } else {
        out
    };
    // RFC 3986 §5.2.4 : un segment final `.`/`..` laisse un
    // slash terminal (`/a/.` → `/a/`, `a/b/..` → `a/`). Sans
    // lui `new URL(".", "…/a/b")` rendait `…/a` — la résolution
    // d'assets relative svelte/fastapi/mkdocs cassait.
    if !out.is_empty() && !out.ends_with('/') && (path.ends_with("/.") || path.ends_with("/..")) {
        out.push('/');
    }
    out
}

/// Pré-traitement spec de l'input : retrait de TOUS les
/// `\t`/`\n`/`\r` (pas seulement aux bords — `trim()` ne
/// suffisait pas, §B-116) puis trim C0/space des deux bouts.
fn url_clean(s: &str) -> String {
    s.chars()
        .filter(|c| !matches!(c, '\t' | '\n' | '\r'))
        .collect::<String>()
        .trim()
        .to_string()
}

/// Parse absolu `scheme:…` — `None` si pas de schéma.
pub(crate) fn url_parse_absolute(s: &str) -> Option<UrlParts> {
    let cleaned = url_clean(s);
    let s = cleaned.as_str();
    let colon = s.find(':')?;
    let scheme = &s[..colon];
    if scheme.is_empty()
        || !scheme
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
        || !scheme
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic())
    {
        return None;
    }
    let mut p = UrlParts {
        scheme: scheme.to_ascii_lowercase(),
        user: String::new(),
        pass: String::new(),
        host: String::new(),
        port: String::new(),
        path: String::new(),
        query: String::new(),
        frag: String::new(),
    };
    let mut rest = &s[colon + 1..];
    if let Some(a) = rest.strip_prefix("//") {
        rest = a;
        let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
        url_split_authority(&rest[..end], &mut p);
        rest = &rest[end..];
    }
    let path_end = rest.find(['?', '#']).unwrap_or(rest.len());
    p.path = url_norm_path(&rest[..path_end]);
    rest = &rest[path_end..];
    if let Some(r) = rest.strip_prefix('?') {
        let qend = r.find('#').unwrap_or(r.len());
        p.query = r[..qend].to_string();
        rest = &r[qend..];
    }
    if let Some(f) = rest.strip_prefix('#') {
        p.frag = f.to_string();
    }
    Some(p)
}

/// Résolution relative basique (WHATWG `basic URL parser` subset) :
/// `//h`, `/p`, `?q`, `#f`, chemin relatif depuis le dossier de base.
pub(crate) fn url_parse(href: &str, base: Option<&UrlParts>) -> Option<UrlParts> {
    let cleaned = url_clean(href);
    let s = cleaned.as_str();
    if let Some(p) = url_parse_absolute(s) {
        return Some(p);
    }
    let b = base?;
    let mut p = UrlParts {
        scheme: b.scheme.clone(),
        user: b.user.clone(),
        pass: b.pass.clone(),
        host: b.host.clone(),
        port: b.port.clone(),
        path: b.path.clone(),
        query: b.query.clone(),
        frag: String::new(),
    };
    if s.is_empty() {
        p.frag.clone_from(&b.frag);
        return Some(p);
    }
    if let Some(f) = s.strip_prefix('#') {
        p.frag = f.to_string();
        return Some(p);
    }
    p.frag.clear();
    if let Some(q) = s.strip_prefix('?') {
        p.query = q.to_string();
        return Some(p);
    }
    p.query.clear();
    if let Some(a) = s.strip_prefix("//") {
        let end = a.find(['/', '?', '#']).unwrap_or(a.len());
        url_split_authority(&a[..end], &mut p);
        let rest = &a[end..];
        let path_end = rest.find(['?', '#']).unwrap_or(rest.len());
        p.path = url_norm_path(&rest[..path_end]);
        let rest = &rest[path_end..];
        if let Some(r) = rest.strip_prefix('?') {
            let qend = r.find('#').unwrap_or(r.len());
            p.query = r[..qend].to_string();
            if let Some(f) = r[qend..].strip_prefix('#') {
                p.frag = f.to_string();
            }
        } else if let Some(f) = rest.strip_prefix('#') {
            p.frag = f.to_string();
        }
        return Some(p);
    }
    // Chemin : absolu (`/p`) ou relatif au dossier de base.
    let path_end = s.find(['?', '#']).unwrap_or(s.len());
    let raw = &s[..path_end];
    p.path = if raw.starts_with('/') {
        url_norm_path(raw)
    } else {
        let dir = match b.path.rfind('/') {
            Some(i) => &b.path[..=i],
            None => "/",
        };
        url_norm_path(&format!("{dir}{raw}"))
    };
    let rest = &s[path_end..];
    if let Some(r) = rest.strip_prefix('?') {
        let qend = r.find('#').unwrap_or(r.len());
        p.query = r[..qend].to_string();
        if let Some(f) = r[qend..].strip_prefix('#') {
            p.frag = f.to_string();
        }
    } else if let Some(f) = rest.strip_prefix('#') {
        p.frag = f.to_string();
    }
    Some(p)
}

/// Encode-sets WHATWG par composant (§B-116) — percent-encode
/// UTF-8 de tout octet hors ensemble. `%` n'appartient à
/// aucun encode-set → les échappements existants sont
/// préservés, pas de double-encodage.
fn url_enc(s: &str, extra: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for &b in s.as_bytes() {
        let keep = matches!(b, 0x21..=0x7E)
            && !matches!(b, b' ' | b'"' | b'<' | b'>' | b'#')
            && !extra.contains(b as char);
        if keep {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    // UTF-8 : les octets > 0x7E tombent dans le `else` —
    // percent-encodés octet par octet (spec).
    out
}

fn url_enc_path(s: &str) -> String {
    url_enc(s, "?`{}")
}

fn url_enc_query(s: &str) -> String {
    url_enc(s, "'")
}

fn url_enc_frag(s: &str) -> String {
    url_enc(s, "`")
}

fn url_enc_userinfo(s: &str) -> String {
    url_enc(s, "?`{}/:;=@[\\]^|")
}

pub(crate) fn url_href(p: &UrlParts) -> String {
    let mut out = format!("{}:", p.scheme);
    // `file:` garde son authority vide (`file:///x`) — spec.
    if !p.host.is_empty() || !p.user.is_empty() || p.scheme == "file" {
        out.push_str("//");
        if !p.user.is_empty() {
            out.push_str(&url_enc_userinfo(&p.user));
            if !p.pass.is_empty() {
                out.push(':');
                out.push_str(&url_enc_userinfo(&p.pass));
            }
            out.push('@');
        }
        out.push_str(&p.host);
        if !p.port.is_empty() {
            out.push(':');
            out.push_str(&p.port);
        }
    }
    if p.path.is_empty() && url_has_origin(&p.scheme) {
        out.push('/');
    } else {
        out.push_str(&url_enc_path(&p.path));
    }
    if !p.query.is_empty() {
        out.push('?');
        out.push_str(&url_enc_query(&p.query));
    }
    if !p.frag.is_empty() {
        out.push('#');
        out.push_str(&url_enc_frag(&p.frag));
    }
    out
}

/// `URL.canParse(url, base?)` — spec : bool, jamais de jeté
/// (les feature-detects modernes l'utilisent avant `new`).
pub(crate) fn url_can_parse(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    Ok(Value::Bool(url_ctor(e, Value::Undefined, args).is_ok()))
}

/// `URL.parse(url, base?)` — spec : instance ou `null`
/// (rejet silencieux du constructeur).
pub(crate) fn url_parse_static(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    match url_ctor(e, Value::Undefined, args) {
        Ok(v) => Ok(v),
        Err(_) => Ok(Value::Null),
    }
}

/// Props cachées (PUA — jamais nommables depuis JS) :
/// `\u{f131}urlh` = href canonique, **source de vérité
/// unique** de l'instance (JS-V1/V-2 — les props `URL` sont
/// des accesseurs proto : getters re-parsent, setters
/// réécrivent). `\u{f131}urls` = l'objet `searchParams`
/// caché (spec : une instance par URL, liée vivante).
/// `\u{f131}urlpar` = lien retour `URLSearchParams → URL`
/// posé par `url_ctor` — `usp_write` réécrit la query du
/// parent à chaque mutation.
const URL_HREF: &str = "\u{f131}urlh";
const URL_SP: &str = "\u{f131}urls";
const USP_PARENT: &str = "\u{f131}urlpar";

/// `origin` sérialisé — `scheme://host[:port]` ou `"null"`
/// (origine opaque : `file:`/pas d'hôte/schèmes non hiérar.).
fn url_origin_str(p: &UrlParts) -> String {
    if url_has_origin(&p.scheme) && p.scheme != "file" && !p.host.is_empty() {
        let mut o = format!("{}://{}", p.scheme, p.host);
        if !p.port.is_empty() {
            o.push(':');
            o.push_str(&p.port);
        }
        o
    } else {
        "null".to_string()
    }
}

/// `this` → parts re-parsées depuis le href caché (les
/// accesseurs vivent sur `URL.prototype` — `this` sans
/// `\u{f131}urlh` = receveur incompatible, spec `TypeError`).
fn url_this(e: &mut Engine, this: Value) -> JsResult<(crate::value::ObjId, UrlParts)> {
    let Value::Obj(id) = this else {
        return Err(e.type_error("URL accessor on non-object"));
    };
    let h = e
        .get_prop(id, URL_HREF)
        .and_then(|v| e.to_js_string(v).ok())
        .unwrap_or_default();
    let Some(p) = url_parse_absolute(&h) else {
        return Err(e.type_error("URL accessor on incompatible receiver"));
    };
    Ok((id, p))
}

/// Écrit le href canonique + resynchronise `searchParams`
/// (lien vivant — V-1). `usp_write` → sync parent
/// re-sérialise la query encodée : le href du setter (query
/// brute conservée, spec `u.search = '?a=%20'`) est rétabli.
fn url_store(e: &mut Engine, id: crate::value::ObjId, p: &UrlParts) {
    let h = e.str_val(url_href(p));
    e.set_own(id, URL_HREF, h, false);
    if let Some(Value::Obj(sp)) = e.get_prop(id, URL_SP) {
        usp_write(e, sp, url_decode_pairs(&p.query));
        let h = e.str_val(url_href(p));
        e.set_own(id, URL_HREF, h, false);
    }
}

/// Setter générique : lit `this`, applique `f` aux parts,
/// réécrit. `f` rend `Err(msg)` → `TypeError` nommant l'input.
fn url_set(
    e: &mut Engine,
    this: Value,
    args: &[Value],
    f: impl Fn(&mut UrlParts, &str) -> Result<(), String>,
) -> JsResult<Value> {
    let (id, mut p) = url_this(e, this)?;
    let raw = e.to_js_string(arg(args, 0))?;
    if let Err(m) = f(&mut p, &raw) {
        return Err(e.type_error(m));
    }
    url_store(e, id, &p);
    Ok(Value::Undefined)
}

pub(crate) fn url_ctor(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let href = e.to_js_string(arg(args, 0))?;
    // `base` peut être une `URL` (lit son `href`) ou une string.
    let base_s = match arg(args, 1) {
        Value::Undefined => None,
        v => Some(e.to_js_string(v)?),
    };
    let base = base_s.as_deref().and_then(|b| url_parse_absolute(b.trim()));
    if base_s.is_some() && base.is_none() {
        let b: String = base_s.unwrap_or_default().chars().take(80).collect();
        if tracing::enabled!(target: "dom_probe", tracing::Level::INFO) {
            tracing::info!(target: "dom_probe", site = "url.ctor.fail", arm = "base", input = %href.chars().take(80).collect::<String>(), base = %b);
        }
        return Err(e.type_error(format!("Invalid base URL: {b}")));
    }
    let Some(p) = url_parse(&href, base.as_ref()) else {
        // L'input fautif nommé — la console terrain identifie
        // la valeur rejetée (bras `Invalid URL` ×5 sites =
        // `new URL(currentScript.src)` turbopack : `""` sur
        // script inline — reproduction fidèle du bug Next.js,
        // identique sous Chrome dans le même ordonnancement).
        let h: String = href.chars().take(80).collect();
        if tracing::enabled!(target: "dom_probe", tracing::Level::INFO) {
            // §B-119 : la stack JS nomme le producteur de
            // l'input fautif (frames bornées 8, aplaties sur
            // une ligne — `src:line:col` par bundle).
            let stack: String = e
                .stack_text("", "")
                .lines()
                .map(str::trim)
                .filter(|l| l.starts_with("at "))
                .take(8)
                .collect::<Vec<_>>()
                .join(" <- ");
            let stack: String = stack.chars().take(600).collect();
            // §B-119 : `document.currentScript` au moment du
            // jeté — nomme l'élément en cours d'évaluation
            // (`<script src>` ou inline `<none>`) pour
            // trancher « currentScript périmé vs producteur
            // amont de `""` » (pattern `getAssetPrefix`
            // Next.js : `new URL(currentScript.src)`).
            let cur = e.dom_ctx.as_ref().map_or_else(
                || "<no-dom>".to_string(),
                |(dom, _)| {
                    let d = dom.borrow();
                    e.hooks.current_script.map_or_else(
                        || "<null>".to_string(),
                        |n| {
                            let s = d
                                .attr(n, "src")
                                .unwrap_or("<inline>")
                                .chars()
                                .take(120)
                                .collect::<String>();
                            format!("{}:{}", d.nodes[n as usize].tag, s)
                        },
                    )
                },
            );
            tracing::info!(target: "dom_probe", site = "url.ctor.fail", arm = "input", input = %h, base = %base_s.as_deref().unwrap_or("<absent>").chars().take(80).collect::<String>(), cur = %cur, stack = %stack);
        }
        return Err(e.type_error(format!("Invalid URL: {h}")));
    };
    let proto = Value::Obj(e.protos.url);
    let id = e.heap.alloc_obj(crate::heap::Object::ordinary(proto));
    // JS-V1/V-2 — props = accesseurs proto : le href
    // canonique est la vérité unique, `searchParams` lié.
    let h = e.str_val(url_href(&p));
    e.set_own(id, URL_HREF, h, false);
    let sp = usp_new(e, url_decode_pairs(&p.query));
    if let Value::Obj(spid) = sp {
        e.set_own(spid, USP_PARENT, Value::Obj(id), false);
    }
    e.set_own(id, URL_SP, sp, false);
    Ok(Value::Obj(id))
}

/// `toString`/`toJSON` — sérialisation courante (accesseur
/// `href` : la vérité cachée, jamais un snapshot).
pub(crate) fn url_href_get(e: &mut Engine, this: Value, a: &[Value]) -> JsResult<Value> {
    url_acc_href(e, this, a)
}

// -- Accesseurs `URL.prototype` (JS-V1/V-2) ----------------------------
// Spec WHATWG : toutes les props sont des getter/setter sur le
// proto (énumérables — `for in`/descriptors de feature-detect
// les voient). Chaque accès re-parse le href caché — source de
// vérité unique, jamais de divergence part/prop.

/// Getter générique : `this` → parts → champ sérialisé.
fn url_get(e: &mut Engine, this: Value, f: impl Fn(&UrlParts) -> String) -> JsResult<Value> {
    let (_id, p) = url_this(e, this)?;
    Ok(e.str_val(f(&p)))
}

pub(crate) fn url_acc_href(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    url_get(e, this, url_href)
}
pub(crate) fn url_acc_protocol(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    url_get(e, this, |p| format!("{}:", p.scheme))
}
pub(crate) fn url_acc_origin(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    url_get(e, this, url_origin_str)
}
pub(crate) fn url_acc_username(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    url_get(e, this, |p| p.user.clone())
}
pub(crate) fn url_acc_password(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    url_get(e, this, |p| p.pass.clone())
}
pub(crate) fn url_acc_host(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    url_get(e, this, |p| {
        if p.port.is_empty() {
            p.host.clone()
        } else {
            format!("{}:{}", p.host, p.port)
        }
    })
}
pub(crate) fn url_acc_hostname(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    url_get(e, this, |p| p.host.clone())
}
pub(crate) fn url_acc_port(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    url_get(e, this, |p| p.port.clone())
}
pub(crate) fn url_acc_pathname(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    url_get(e, this, |p| {
        if p.path.is_empty() && url_has_origin(&p.scheme) {
            "/".to_string()
        } else {
            p.path.clone()
        }
    })
}
pub(crate) fn url_acc_search(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    url_get(e, this, |p| {
        if p.query.is_empty() {
            String::new()
        } else {
            format!("?{}", p.query)
        }
    })
}
pub(crate) fn url_acc_hash(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    url_get(e, this, |p| {
        if p.frag.is_empty() {
            String::new()
        } else {
            format!("#{}", p.frag)
        }
    })
}
/// `searchParams` — instance cachée liée vivante (V-1).
pub(crate) fn url_acc_search_params(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    let (id, _p) = url_this(e, this)?;
    Ok(e.get_prop(id, URL_SP).unwrap_or(Value::Undefined))
}

pub(crate) fn url_set_href(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    url_set(e, this, args, |p, raw| match url_parse_absolute(raw) {
        Some(np) => {
            *p = np;
            Ok(())
        }
        None => Err(format!(
            "Invalid URL: {}",
            raw.chars().take(80).collect::<String>()
        )),
    })
}
pub(crate) fn url_set_protocol(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    url_set(e, this, args, |p, raw| {
        // Spec : ignorer silencieusement un schéma invalide.
        let s = raw.strip_suffix(':').unwrap_or(raw);
        if !s.is_empty()
            && s.chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
            && s.chars().next().is_some_and(|c| c.is_ascii_alphabetic())
        {
            p.scheme = s.to_ascii_lowercase();
        }
        Ok(())
    })
}
pub(crate) fn url_set_username(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    url_set(e, this, args, |p, raw| {
        p.user = raw.to_string();
        Ok(())
    })
}
pub(crate) fn url_set_password(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    url_set(e, this, args, |p, raw| {
        p.pass = raw.to_string();
        Ok(())
    })
}
pub(crate) fn url_set_host(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    url_set(e, this, args, |p, raw| {
        url_split_authority(raw, p);
        Ok(())
    })
}
pub(crate) fn url_set_hostname(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    url_set(e, this, args, |p, raw| {
        url_split_authority(raw, p);
        Ok(())
    })
}
pub(crate) fn url_set_port(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    url_set(e, this, args, |p, raw| {
        if raw.is_empty() {
            p.port.clear();
        } else if raw.chars().all(|c| c.is_ascii_digit()) {
            let default = URL_DEFAULT_PORTS
                .iter()
                .find(|(s, _)| s == &p.scheme.as_str())
                .map_or("", |(_, d)| *d);
            p.port = if raw == default {
                String::new()
            } else {
                raw.to_string()
            };
        }
        Ok(())
    })
}
pub(crate) fn url_set_pathname(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    url_set(e, this, args, |p, raw| {
        // Spec : chemin opaque (`data:`/`mailto:`…) non
        // assignable — ignoré, pas de jeté.
        if url_has_origin(&p.scheme) || p.path.starts_with('/') {
            let np = if raw.starts_with('/') {
                raw.to_string()
            } else {
                format!("/{raw}")
            };
            p.path = url_norm_path(&np);
        }
        Ok(())
    })
}
pub(crate) fn url_set_search(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    url_set(e, this, args, |p, raw| {
        p.query = raw.strip_prefix('?').unwrap_or(raw).to_string();
        Ok(())
    })
}
pub(crate) fn url_set_hash(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    url_set(e, this, args, |p, raw| {
        p.frag = raw.strip_prefix('#').unwrap_or(raw).to_string();
        Ok(())
    })
}

// -- URLSearchParams (x-www-form-urlencoded) --------------------------

/// Paire `(name, value)` encodée formulaire — `+` = espace.
pub(crate) fn url_decode_pairs(q: &str) -> Vec<(String, String)> {
    if q.is_empty() {
        return Vec::new();
    }
    q.split('&')
        .filter(|seg| !seg.is_empty())
        .map(|seg| {
            let (k, v) = seg.split_once('=').unwrap_or((seg, ""));
            (form_decode(k), form_decode(v))
        })
        .collect()
}

pub(crate) fn form_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => out.push(b' '),
            b'%' if i + 2 < bytes.len() => {
                let hex = |b: u8| (b as char).to_digit(16);
                match (hex(bytes[i + 1]), hex(bytes[i + 2])) {
                    (Some(h), Some(l)) => {
                        out.push((h * 16 + l) as u8);
                        i += 2;
                    }
                    _ => out.push(b'%'),
                }
            }
            b => out.push(b),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

pub(crate) fn form_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for &b in s.as_bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'*' | b'-' | b'.' | b'_' => {
                out.push(b as char);
            }
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

const USP_PAIRS: &str = "\\u{f131}urlp";

/// Lit les paires cachées `\u{f131}urlp` (array `[ [k,v],… ]`).
pub(crate) fn usp_pairs(e: &mut Engine, id: crate::value::ObjId) -> Vec<(String, String)> {
    let Some(Value::Obj(arr)) = e.get_prop(id, USP_PAIRS) else {
        return Vec::new();
    };
    let n = match e.get_prop(arr, "length") {
        Some(Value::Num(n)) => n.max(0.0) as usize,
        _ => 0,
    };
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        if let Some(Value::Obj(pair)) = e.get_prop(arr, &i.to_string()) {
            let k = e
                .get_prop(pair, "0")
                .map(|v| e.to_js_string(v).unwrap_or_default())
                .unwrap_or_default();
            let v = e
                .get_prop(pair, "1")
                .map(|v| e.to_js_string(v).unwrap_or_default())
                .unwrap_or_default();
            out.push((k, v));
        }
    }
    out
}

/// `v` est-il un `URLSearchParams` ? → ses paires (détection par
/// le prop sentinelle — un `toString` redéfini ne leurre pas la
/// sérialisation `BodyInit`, JS-C67).
pub(crate) fn usp_pairs_of(e: &mut Engine, v: Value) -> Option<Vec<(String, String)>> {
    let Value::Obj(id) = v else { return None };
    if !matches!(e.get_prop(id, USP_PAIRS), Some(Value::Obj(_))) {
        return None;
    }
    Some(usp_pairs(e, id))
}

pub(crate) fn usp_write(e: &mut Engine, id: crate::value::ObjId, pairs: Vec<(String, String)>) {
    let vals: Vec<Value> = pairs
        .into_iter()
        .map(|(k, v)| {
            let k = e.str_val(k);
            let v = e.str_val(v);
            e.new_array(vec![k, v])
        })
        .collect();
    let arr = e.new_array(vals);
    e.set_own(id, USP_PAIRS, arr, false);
    // JS-V1 — `u.searchParams` lié vivant : chaque mutation
    // réécrit la query du parent (spec : même liste — les
    // paires sont sérialisées encodées formulaire).
    if let Some(Value::Obj(pid)) = e.get_prop(id, USP_PARENT) {
        let pairs = usp_pairs(e, id);
        if let Some(hv) = e.get_prop(pid, URL_HREF) {
            let h = e.to_js_string(hv).unwrap_or_default();
            if let Some(mut p) = url_parse_absolute(&h) {
                p.query = pairs
                    .iter()
                    .map(|(k, v)| format!("{}={}", form_encode(k), form_encode(v)))
                    .collect::<Vec<_>>()
                    .join("&");
                let hv = e.str_val(url_href(&p));
                e.set_own(pid, URL_HREF, hv, false);
            }
        }
    }
}

/// Nouvel objet `URLSearchParams` (proto `protos.url_params`).
pub(crate) fn usp_new(e: &mut Engine, pairs: Vec<(String, String)>) -> Value {
    let proto = Value::Obj(e.protos.url_params);
    let id = e.heap.alloc_obj(crate::heap::Object::ordinary(proto));
    usp_write(e, id, pairs);
    Value::Obj(id)
}

pub(crate) fn usp_this(
    e: &mut Engine,
    this: Value,
) -> JsResult<(crate::value::ObjId, Vec<(String, String)>)> {
    let Value::Obj(id) = this else {
        return Err(e.type_error("URLSearchParams method on non-object"));
    };
    if !matches!(e.get_prop(id, USP_PAIRS), Some(Value::Obj(_))) {
        return Err(e.type_error("URLSearchParams method on incompatible receiver"));
    }
    Ok((id, usp_pairs(e, id)))
}

pub(crate) fn usp_ctor(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let mut pairs = Vec::new();
    match arg(args, 0) {
        Value::Undefined | Value::Null => {}
        v @ Value::Obj(oid) => {
            // Iterable (`[[k,v],…]`, autre URLSearchParams via son
            // `[Symbol.iterator]`) → paires ; non-itérable → record
            // `{k: v}` dans l'ordre des props énumérables.
            match e.iter_values(v.clone()) {
                Ok(items) => {
                    for it in items {
                        // Spec `sequence<sequence>` : chaque item
                        // doit être une paire exacte — scalaire ou
                        // tuple ≠ 2 → TypeError (feature-detects).
                        if !matches!(it, Value::Obj(_)) {
                            return Err(
                                e.type_error("URLSearchParams: sequence item is not a pair")
                            );
                        }
                        let (k, val) = match e.iter_values(it.clone()) {
                            Ok(ivs) if ivs.len() == 2 => (ivs[0].clone(), ivs[1].clone()),
                            Ok(_) => {
                                return Err(e.type_error(
                                    "URLSearchParams: pair must have exactly 2 elements",
                                ));
                            }
                            // Non-itérable : l'objet indexé 0/1
                            // reste accepté (permissif — la spec
                            // jetterait aussi ici).
                            Err(_) => (
                                e.get_prop_value(it.clone(), "0")?,
                                e.get_prop_value(it, "1")?,
                            ),
                        };
                        pairs.push((e.to_js_string(k)?, e.to_js_string(val)?));
                    }
                }
                Err(_) => {
                    if matches!(e.get_prop(oid, USP_PAIRS), Some(Value::Obj(_))) {
                        // Copie directe si l'itérateur a été cassé.
                        pairs = usp_pairs(e, oid);
                    } else {
                        for k in e.own_enum_keys(oid) {
                            let val = e.get_prop_value(v.clone(), &k)?;
                            pairs.push((k, e.to_js_string(val)?));
                        }
                    }
                }
            }
        }
        v => {
            let s = e.to_js_string(v)?;
            let q = s.strip_prefix('?').unwrap_or(&s);
            pairs = url_decode_pairs(q);
        }
    }
    Ok(usp_new(e, pairs))
}

pub(crate) fn usp_get(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let (_id, pairs) = usp_this(e, this)?;
    let name = e.to_js_string(arg(args, 0))?;
    match pairs.iter().find(|(k, _)| *k == name) {
        Some((_, v)) => Ok(e.str_val(v.clone())),
        None => Ok(Value::Null),
    }
}

pub(crate) fn usp_get_all(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let (_id, pairs) = usp_this(e, this)?;
    let name = e.to_js_string(arg(args, 0))?;
    let vals: Vec<Value> = pairs
        .iter()
        .filter(|(k, _)| *k == name)
        .map(|(_, v)| e.str_val(v.clone()))
        .collect();
    Ok(e.new_array(vals))
}

/// `.size` (spec 2023) — nombre de paires, accesseur proto.
pub(crate) fn usp_size(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    let (_id, pairs) = usp_this(e, this)?;
    Ok(Value::Num(pairs.len() as f64))
}

pub(crate) fn usp_has(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let (_id, pairs) = usp_this(e, this)?;
    let name = e.to_js_string(arg(args, 0))?;
    // Spec 2023 : `has(name, value)` filtre aussi la valeur —
    // l'arg est `optional` : `has('a', undefined)` filtre la
    // chaîne "undefined" (ne PAS confondre absent/undefined,
    // §B-116).
    let val_filter = match args.get(1) {
        None => None,
        Some(v) => Some(e.to_js_string(v.clone())?),
    };
    Ok(Value::Bool(pairs.iter().any(|(k, v)| {
        *k == name && val_filter.as_ref().is_none_or(|f| f == v)
    })))
}

pub(crate) fn usp_set(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let (id, mut pairs) = usp_this(e, this)?;
    let name = e.to_js_string(arg(args, 0))?;
    let val = e.to_js_string(arg(args, 1))?;
    let mut out = Vec::with_capacity(pairs.len());
    let mut inserted = false;
    for (k, v) in pairs.drain(..) {
        if k == name {
            if !inserted {
                out.push((k, val.clone()));
                inserted = true;
            }
        } else {
            out.push((k, v));
        }
    }
    if !inserted {
        out.push((name, val));
    }
    usp_write(e, id, out);
    Ok(Value::Undefined)
}

pub(crate) fn usp_append(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let (id, mut pairs) = usp_this(e, this)?;
    let name = e.to_js_string(arg(args, 0))?;
    let val = e.to_js_string(arg(args, 1))?;
    pairs.push((name, val));
    usp_write(e, id, pairs);
    Ok(Value::Undefined)
}

pub(crate) fn usp_delete(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let (id, pairs) = usp_this(e, this)?;
    let name = e.to_js_string(arg(args, 0))?;
    // `delete(name, optional value)` — `undefined` explicite
    // filtre la chaîne "undefined" (§B-116, comme `has`).
    let val_filter = match args.get(1) {
        None => None,
        Some(v) => Some(e.to_js_string(v.clone())?),
    };
    let keep: Vec<(String, String)> = pairs
        .into_iter()
        .filter(|(k, v)| !(k == &name && val_filter.as_ref().is_none_or(|f| f == v)))
        .collect();
    usp_write(e, id, keep);
    Ok(Value::Undefined)
}

pub(crate) fn usp_sort(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    let (id, mut pairs) = usp_this(e, this)?;
    // Tri stable sur la clé (spec : code points).
    pairs.sort_by(|a, b| a.0.cmp(&b.0));
    usp_write(e, id, pairs);
    Ok(Value::Undefined)
}

pub(crate) fn usp_for_each(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let (id, _pairs) = usp_this(e, this.clone())?;
    let cb = arg(args, 0);
    if !e.is_callable(cb.clone()) {
        return Err(e.type_error("forEach needs a function"));
    }
    let this_arg = arg(args, 1);
    // Spec : itération de la liste VIVANTE par index — la cb
    // peut muter (append/delete) et le pas suivant re-lit
    // (`delete('b')` pendant le pas 0 saute 'b', §B-116).
    let mut i = 0;
    loop {
        let pairs = usp_pairs(e, id);
        if i >= pairs.len() {
            break;
        }
        let (k, v) = pairs[i].clone();
        let kv = e.str_val(k);
        let vv = e.str_val(v);
        e.call_value(cb.clone(), this_arg.clone(), &[vv, kv, this.clone()])?;
        i += 1;
    }
    Ok(Value::Undefined)
}

pub(crate) fn usp_to_string(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    let (_id, pairs) = usp_this(e, this)?;
    let out: Vec<String> = pairs
        .iter()
        .map(|(k, v)| format!("{}={}", form_encode(k), form_encode(v)))
        .collect();
    Ok(e.str_val(out.join("&")))
}

/// Itérateur USP vivant — `next` re-lit `usp_pairs` du parent
/// à chaque pas (spec : append/delete pendant l'itération
/// visibles, §B-116). L'état `{p,i,k}` vit dans un objet
/// interne passé en bound-arg — aucun `ObjKind` nouveau
/// (les props sont tracées par le GC comme toutes les autres).
const USPI_P: &str = "\u{f131}uspp";
const USPI_I: &str = "\u{f131}uspi";
const USPI_K: &str = "\u{f131}uspk";

fn usp_iter_live(e: &mut Engine, parent: Value, kind: &str) -> Value {
    let state = e.new_object();
    e.set_own(state, USPI_P, parent, false);
    e.set_own(state, USPI_I, Value::Num(0.0), false);
    let k = e.str_val(kind);
    e.set_own(state, USPI_K, k, false);
    let next = e.bound_native("next", usp_iter_next, vec![Value::Obj(state)]);
    let it = e
        .heap
        .alloc_obj(crate::heap::Object::ordinary(Value::Obj(e.protos.usp_iter)));
    e.set_own(it, "next", next, true);
    Value::Obj(it)
}

/// `next` de l'itérateur vivant — `bound[0]` = l'état.
fn usp_iter_next(e: &mut Engine, _t: Value, a: &[Value]) -> JsResult<Value> {
    let state = arg(a, 0);
    let parent = e
        .get_prop_value(state.clone(), USPI_P)
        .unwrap_or(Value::Undefined);
    let i = match e.get_prop_value(state.clone(), USPI_I) {
        Ok(Value::Num(n)) => n as usize,
        _ => 0,
    };
    let kind = e
        .get_prop_value(state.clone(), USPI_K)
        .ok()
        .and_then(|v| e.to_js_string(v).ok())
        .unwrap_or_default();
    let pairs = match parent {
        Value::Obj(pid) => usp_pairs(e, pid),
        _ => Vec::new(),
    };
    let done = i >= pairs.len();
    let mut v = Value::Undefined;
    if !done {
        if let Value::Obj(sid) = state.clone() {
            let _ = e.set_prop(sid, USPI_I, Value::Num((i + 1) as f64));
        }
        let (k, val) = pairs[i].clone();
        v = match kind.as_str() {
            "keys" => e.str_val(k),
            "values" => e.str_val(val),
            _ => {
                let k = e.str_val(k);
                let val = e.str_val(val);
                e.new_array(vec![k, val])
            }
        };
    }
    let o = e.new_object();
    e.set_own(o, "value", v, true);
    e.set_own(o, "done", Value::Bool(done), true);
    Ok(Value::Obj(o))
}

pub(crate) fn usp_keys(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    let (_id, _pairs) = usp_this(e, this.clone())?;
    Ok(usp_iter_live(e, this, "keys"))
}

pub(crate) fn usp_values(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    let (_id, _pairs) = usp_this(e, this.clone())?;
    Ok(usp_iter_live(e, this, "values"))
}

pub(crate) fn usp_entries(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    let (_id, _pairs) = usp_this(e, this.clone())?;
    Ok(usp_iter_live(e, this, "entries"))
}
