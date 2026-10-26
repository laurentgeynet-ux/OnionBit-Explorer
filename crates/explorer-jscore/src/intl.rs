//! `Intl` subset — fixed `en-US` locale, UTC only (no CLDR data
//! vendored — documented deviation). Implemented: `DateTimeFormat`,
//! `NumberFormat`, `Collator`, `PluralRules`, `ListFormat`,
//! `RelativeTimeFormat`, `Locale`, `DisplayNames`,
//! `getCanonicalLocales`, `supportedLocalesOf`. The formatter
//! options are stored as a `k=v\u{1}k=v` blob on the instance
//! (`__opts`); `format`/`formatToParts`/`resolvedOptions` are own
//! bound natives so destructuring them keeps `this` (spec).

use crate::builtins::arg;
use crate::heap::{JsResult, Object};
use crate::interp::Engine;
use crate::value::{ObjId, Value};

/// Installe l'objet `Intl` et ses constructeurs sur le global.
pub(crate) fn install(e: &mut Engine) {
    let intl = e.new_object();
    let intl_v = Value::Obj(intl);
    e.add_method(intl, "getCanonicalLocales", canonical);
    e.add_method(intl, "supportedLocalesOf", canonical);
    e.add_method(intl, "supportedValuesOf", supported_values_of);
    for (name, f) in [
        ("DateTimeFormat", intl_dtf_ctor as crate::heap::NativeFn),
        ("NumberFormat", intl_nf_ctor),
        ("Collator", intl_collator_ctor),
        ("PluralRules", intl_plural_ctor),
        ("ListFormat", intl_list_ctor),
        ("RelativeTimeFormat", intl_rtf_ctor),
        ("Locale", intl_locale_ctor),
        ("DisplayNames", intl_display_ctor),
        ("Segmenter", intl_segmenter_ctor),
    ] {
        let p = e.new_object();
        let c = crate::builtins::ctor(e, name, f, p);
        e.set_own(intl, name, c, false);
        // `supportedLocalesOf` statique (spec) sur chaque ctor.
        e.add_method(crate::builtins::c_id(c), "supportedLocalesOf", canonical);
    }
    let tag = e.str_val("Intl");
    e.set_own(intl, crate::interp::SYM_TO_STRING_TAG, tag, false);
    e.set_own(e.global, "Intl", intl_v, false);
    install_locale_accessors(e);
}

// -- helpers communs ---------------------------------------------------

/// Proto de `Intl.<name>` (lu dynamiquement — `ctor` a posé
/// `prototype` sur la fonction).
fn ctor_proto(e: &mut Engine, name: &str) -> Value {
    e.get_prop(e.global, "Intl")
        .and_then(|i| match i {
            Value::Obj(iid) => e.get_prop(iid, name),
            _ => None,
        })
        .and_then(|c| match c {
            Value::Obj(cid) => e.get_prop(cid, "prototype"),
            _ => None,
        })
        .unwrap_or(Value::Obj(e.protos.object))
}

/// Instance `Intl` : objet ordinaire + `__opts`/`__locale` blobs +
/// méthodes liées propres (spec : `format` est un getter bound).
fn intl_obj(
    e: &mut Engine,
    proto_name: &str,
    opts: &str,
    binds: &[(&'static str, crate::heap::NativeFn)],
) -> Value {
    let mut o = Object::ordinary(ctor_proto(e, proto_name));
    o.set_own("__opts", e.str_val(opts), false);
    o.set_own("__locale", e.str_val("en-US"), false);
    let id = e.heap.alloc_obj(o);
    let v = Value::Obj(id);
    for (n, f) in binds {
        let bv = e.bound_native(n, *f, vec![v]);
        e.set_own(id, n, bv, false);
    }
    v
}

/// Lit une option `k=v` dans le blob `__opts` (`\u{1}`-séparé).
fn opt<'a>(opts: &'a str, key: &str) -> Option<&'a str> {
    opts.split('\u{1}')
        .find_map(|kv| kv.strip_prefix(key)?.strip_prefix('='))
}

/// Instance `Intl` depuis `args[0]` (méthodes liées) ou `this`.
fn self_id(e: &mut Engine, this: Value, args: &[Value], who: &str) -> JsResult<ObjId> {
    let cand = match arg(args, 0) {
        v @ Value::Obj(_) => v,
        _ => this,
    };
    match cand {
        Value::Obj(id) if e.get_prop(id, "__opts").is_some() => Ok(id),
        _ => Err(e.type_error(format!("Intl method on non-{who}"))),
    }
}

/// Chaîne `__opts` de `id` (blob moteur).
fn self_opts(e: &mut Engine, id: ObjId) -> String {
    match e.get_prop(id, "__opts") {
        Some(Value::Str(sid)) => e.heap.string(sid).unwrap_or_default(),
        _ => String::new(),
    }
}

/// Construit le blob `__opts` à partir des props de l'objet options.
fn read_opts(e: &mut Engine, v: Value, keys: &[&str]) -> JsResult<String> {
    let mut blob = String::new();
    if let Value::Obj(id) = v {
        for k in keys {
            if let Some(pv) = e.get_prop(id, k) {
                if matches!(pv, Value::Undefined) {
                    continue;
                }
                let sv = e.to_js_string(pv)?;
                blob.push_str(k);
                blob.push('=');
                blob.push_str(&sv);
                blob.push('\u{1}');
            }
        }
    }
    Ok(blob)
}

/// ms epoch d'un argument Date-like (`__time` interne ou coercition).
fn date_ms(e: &mut Engine, v: Value) -> JsResult<f64> {
    if let Value::Obj(id) = v {
        if let Some(Value::Num(n)) = e.get_prop(id, "__time") {
            return Ok(n);
        }
    }
    let n = e.to_number(v)?;
    if n.is_nan() {
        return Err(e.range_error("Invalid time value"));
    }
    Ok(n)
}

// -- canonicalisation BCP-47 (sous-ensemble) ---------------------------

/// Canonicalise un tag : langue minuscule, script Titre, région
/// MAJ (`en-us` → `en-US`, `sr-cyrl-rs` → `sr-Cyrl-RS`). Tag
/// invalide → `None` (`RangeError` spec).
fn canonical_tag(tag: &str) -> Option<String> {
    let tag = tag.trim();
    if tag.is_empty() {
        return Some(String::new());
    }
    let mut out: Vec<String> = Vec::new();
    for (i, sub) in tag.split('-').enumerate() {
        if sub.is_empty() || sub.len() > 8 || !sub.chars().all(|c| c.is_ascii_alphanumeric()) {
            return None;
        }
        let s = if i == 0 {
            if !sub.chars().all(|c| c.is_ascii_alphabetic()) {
                return None;
            }
            sub.to_ascii_lowercase()
        } else if sub.len() == 4 && sub.chars().all(|c| c.is_ascii_alphabetic()) {
            let mut c = sub.chars();
            format!(
                "{}{}",
                c.next().unwrap_or_default().to_ascii_uppercase(),
                c.as_str().to_ascii_lowercase()
            )
        } else if (sub.len() == 2 && sub.chars().all(|c| c.is_ascii_alphabetic()))
            || (sub.len() == 3 && sub.chars().all(|c| c.is_ascii_digit()))
        {
            sub.to_ascii_uppercase()
        } else {
            sub.to_ascii_lowercase()
        };
        out.push(s);
    }
    Some(out.join("-"))
}

/// `Intl.getCanonicalLocales(list)` / `*.supportedLocalesOf` — on
/// « supporte » tout via le repli `en-US` (spec autorise tout
/// sous-ensemble).
fn canonical(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let v = arg(args, 0);
    let tags: Vec<String> = match v {
        Value::Str(_) => vec![e.to_js_string(v)?],
        Value::Obj(_) => {
            let mut out = Vec::new();
            for item in e.iter_values(v)? {
                out.push(e.to_js_string(item)?);
            }
            out
        }
        _ => Vec::new(),
    };
    let mut vals = Vec::with_capacity(tags.len());
    for t in tags {
        match canonical_tag(&t) {
            Some(c) if !c.is_empty() => vals.push(e.str_val(c)),
            Some(_) => {}
            None => return Err(e.range_error("Incorrect locale information provided")),
        }
    }
    Ok(e.new_array(vals))
}

/// `Intl.supportedValuesOf(key)` — sous-ensemble honnête : sans
/// données CLDR, on rapporte les valeurs que le moteur traite
/// réellement (`gregory`, `latn`, `UTC`, devises courantes du
/// `NumberFormat`). Clé inconnue → `RangeError` (spec). Mur mesuré :
/// `supportedValuesOf is not a function` tuait le bootstrap Vue de
/// bandcamp.com/discover.
fn supported_values_of(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let key = e.to_js_string(arg(args, 0))?;
    let list: &[&str] = match key.as_str() {
        "calendar" => &["gregory"],
        "collation" => &["default"],
        "currency" => &["USD", "EUR", "GBP", "JPY", "CAD", "AUD", "CHF", "CNY"],
        "numberingSystem" => &["latn", "arab"],
        "timeZone" => &["UTC"],
        "unit" => &[],
        _ => {
            return Err(e.range_error(format!("invalid key: {key}")));
        }
    };
    let vals: Vec<Value> = list.iter().map(|s| e.str_val(*s)).collect();
    Ok(e.new_array(vals))
}

// -- noms communs ------------------------------------------------------

const WEEKDAYS_L: [&str; 7] = [
    "Sunday",
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
];
const WEEKDAYS_S: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
const MONTHS_L: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];
const MONTHS_S: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

// -- DateTimeFormat ----------------------------------------------------

fn intl_dtf_ctor(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let opts = read_opts(
        e,
        arg(args, 1),
        &[
            "dateStyle",
            "timeStyle",
            "weekday",
            "era",
            "year",
            "month",
            "day",
            "dayPeriod",
            "hour",
            "minute",
            "second",
            "fractionalSecondDigits",
            "timeZoneName",
            "hour12",
            "hourCycle",
            "timeZone",
        ],
    )?;
    Ok(intl_obj(
        e,
        "DateTimeFormat",
        &opts,
        &[
            ("format", dtf_format),
            ("formatToParts", dtf_parts),
            ("formatRange", dtf_range),
            ("resolvedOptions", dtf_resolved),
        ],
    ))
}

/// Partie `(type, valeur)` produite par le formatage date.
type DPart = (&'static str, String);

/// Décompose `ms` en parties de formatage selon `opts` (en-US/UTC).
fn dtf_build(e: &mut Engine, opts: &str, ms: f64) -> JsResult<Vec<DPart>> {
    let (y, mo, d, wd, h, mi, s, mss) = crate::builtins::ms_to_parts(ms);
    let mut p: Vec<DPart> = Vec::new();
    let ds = opt(opts, "dateStyle");
    let ts = opt(opts, "timeStyle");
    let hour12 = opt(opts, "hour12").is_none_or(|v| v != "false" && v != "24");

    if let Some(style) = ds {
        // Styles de date en-US.
        match style {
            "full" => {
                p.push(("weekday", WEEKDAYS_L[wd as usize].into()));
                p.push(("literal", ", ".into()));
                p.push(("month", MONTHS_L[mo as usize - 1].into()));
                p.push(("literal", " ".into()));
                p.push(("day", d.to_string()));
                p.push(("literal", ", ".into()));
                p.push(("year", y.to_string()));
            }
            "long" => {
                p.push(("month", MONTHS_L[mo as usize - 1].into()));
                p.push(("literal", " ".into()));
                p.push(("day", d.to_string()));
                p.push(("literal", ", ".into()));
                p.push(("year", y.to_string()));
            }
            "medium" => {
                p.push(("month", MONTHS_S[mo as usize - 1].into()));
                p.push(("literal", " ".into()));
                p.push(("day", d.to_string()));
                p.push(("literal", ", ".into()));
                p.push(("year", y.to_string()));
            }
            _ => {
                // short : M/D/YY.
                p.push(("month", mo.to_string()));
                p.push(("literal", "/".into()));
                p.push(("day", d.to_string()));
                p.push(("literal", "/".into()));
                p.push(("year", format!("{:02}", y.rem_euclid(100))));
            }
        }
    } else if ts.is_none()
        || opt(opts, "weekday").is_some()
        || opt(opts, "year").is_some()
        || opt(opts, "month").is_some()
        || opt(opts, "day").is_some()
    {
        // Format par composants (ordre en-US : weekday, month day, year).
        let has_day = opt(opts, "day").is_some();
        let has_time = opt(opts, "hour").is_some()
            || opt(opts, "minute").is_some()
            || opt(opts, "second").is_some();
        if let Some(w) = opt(opts, "weekday") {
            p.push((
                "weekday",
                match w {
                    "long" => WEEKDAYS_L[wd as usize].into(),
                    "narrow" => WEEKDAYS_L[wd as usize]
                        .chars()
                        .next()
                        .unwrap_or_default()
                        .to_string(),
                    _ => WEEKDAYS_S[wd as usize].into(),
                },
            ));
            p.push(("literal", ", ".into()));
        }
        let num_month = matches!(opt(opts, "month"), Some("numeric" | "2-digit"));
        if let Some(m) = opt(opts, "month") {
            let sep = if num_month { "/" } else { " " };
            p.push((
                "month",
                match m {
                    "2-digit" => format!("{mo:02}"),
                    "long" => MONTHS_L[mo as usize - 1].into(),
                    "short" => MONTHS_S[mo as usize - 1].into(),
                    "narrow" => MONTHS_L[mo as usize - 1]
                        .chars()
                        .next()
                        .unwrap_or_default()
                        .to_string(),
                    _ => mo.to_string(),
                },
            ));
            if has_day || opt(opts, "year").is_some() {
                p.push(("literal", sep.into()));
            }
        }
        if has_day {
            let dv = opt(opts, "day").unwrap_or("numeric");
            p.push((
                "day",
                if dv == "2-digit" {
                    format!("{d:02}")
                } else {
                    d.to_string()
                },
            ));
            if opt(opts, "year").is_some() || has_time {
                let sep = if num_month && opt(opts, "year").is_some() {
                    "/"
                } else {
                    ", "
                };
                p.push(("literal", sep.into()));
            }
        }
        if let Some(yo) = opt(opts, "year") {
            p.push((
                "year",
                if yo == "2-digit" {
                    format!("{:02}", y.rem_euclid(100))
                } else {
                    y.to_string()
                },
            ));
            if has_time {
                p.push(("literal", ", ".into()));
            }
        }
        // Mois seul + année : `January 2000` / `01/2000`.
        if opt(opts, "month").is_some() && !has_day && opt(opts, "year").is_none() && has_time {
            p.push(("literal", ", ".into()));
        }
        // Défaut spec : aucun style ni composant → M/D/YYYY.
        if ts.is_none()
            && opt(opts, "year").is_none()
            && opt(opts, "month").is_none()
            && !has_day
            && !has_time
        {
            p.push(("month", mo.to_string()));
            p.push(("literal", "/".into()));
            p.push(("day", d.to_string()));
            p.push(("literal", "/".into()));
            p.push(("year", y.to_string()));
        }
    }

    if let Some(style) = ts {
        if !p.is_empty() {
            let sep = if ds == Some("short") { ", " } else { " at " };
            p.push(("literal", sep.into()));
        }
        // Styles d'heure en-US : short→"10:30 AM", medium→"10:30:00 AM",
        // long/full→"10:30:00 AM GMT"/"… Coordinated Universal Time".
        let (h12, ampm) = if hour12 && h < 12 {
            (if h == 0 { 12 } else { h }, "AM")
        } else if hour12 {
            (if h == 12 { 12 } else { h - 12 }, "PM")
        } else {
            (h, "")
        };
        p.push(("hour", h12.to_string()));
        p.push(("literal", ":".into()));
        p.push(("minute", format!("{mi:02}")));
        if style != "short" {
            p.push(("literal", ":".into()));
            p.push(("second", format!("{s:02}")));
        }
        if hour12 {
            p.push(("literal", " ".into()));
            p.push(("dayPeriod", ampm.into()));
        }
        match style {
            "long" => {
                p.push(("literal", " ".into()));
                p.push(("timeZoneName", "GMT".into()));
            }
            "full" => {
                p.push(("literal", " ".into()));
                p.push(("timeZoneName", "Coordinated Universal Time".into()));
            }
            _ => {}
        }
    } else if opt(opts, "hour").is_some()
        || opt(opts, "minute").is_some()
        || opt(opts, "second").is_some()
    {
        // Composants d'heure explicites.
        if ds.is_none() && opt(opts, "weekday").is_none() && p.is_empty() {
            // rien avant — format heure seul
        } else {
            p.push(("literal", ", ".into()));
        }
        let show_h = opt(opts, "hour").is_some();
        let show_mi = opt(opts, "minute").is_some() || opt(opts, "second").is_some();
        if show_h {
            let hv = opt(opts, "hour").unwrap_or("numeric");
            let (h12, ampm) = if hour12 && h < 12 {
                (if h == 0 { 12 } else { h }, "AM")
            } else if hour12 {
                (if h == 12 { 12 } else { h - 12 }, "PM")
            } else {
                (h, "")
            };
            p.push((
                "hour",
                if hv == "2-digit" {
                    format!("{h12:02}")
                } else {
                    h12.to_string()
                },
            ));
            if show_mi {
                p.push(("literal", ":".into()));
            }
            let _ = ampm;
        }
        if show_mi {
            // `minute`/`second` restent paddées même en `numeric`
            // (observable V8 : `9:05 AM`).
            p.push(("minute", format!("{mi:02}")));
        }
        if opt(opts, "second").is_some() {
            p.push(("literal", ":".into()));
            p.push(("second", format!("{s:02}")));
        }
        if let Some(fsd) = opt(opts, "fractionalSecondDigits") {
            let n: usize = fsd.parse::<usize>().unwrap_or(0).clamp(1, 3);
            p.push(("literal", ".".into()));
            let fs = format!("{mss:03}");
            p.push(("fractionalSecond", fs[..n].to_string()));
        }
        if hour12 {
            let ampm = if h < 12 { "AM" } else { "PM" };
            p.push(("literal", " ".into()));
            p.push(("dayPeriod", ampm.into()));
        }
        if let Some(tz) = opt(opts, "timeZoneName") {
            p.push(("literal", " ".into()));
            p.push((
                "timeZoneName",
                match tz {
                    "long" | "longGeneric" | "longOffset" => "Coordinated Universal Time".into(),
                    "shortOffset" | "shortGeneric" | "short" => "GMT".into(),
                    _ => "UTC".into(),
                },
            ));
        }
    }
    let _ = e;
    Ok(p)
}

fn dtf_ms(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<(ObjId, String, f64)> {
    let id = self_id(e, this, args, "DateTimeFormat")?;
    let opts = self_opts(e, id);
    let ms = date_ms(e, arg(args, 1))?;
    if ms.is_nan() {
        return Err(e.range_error("Invalid time value"));
    }
    Ok((id, opts, ms))
}

fn dtf_format(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let (_id, opts, ms) = dtf_ms(e, this, args)?;
    let parts = dtf_build(e, &opts, ms)?;
    Ok(e.str_val(parts.iter().map(|(_, v)| v.as_str()).collect::<String>()))
}

fn dtf_parts(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let (_id, opts, ms) = dtf_ms(e, this, args)?;
    let parts = dtf_build(e, &opts, ms)?;
    let mut out = Vec::with_capacity(parts.len());
    for (ty, val) in parts {
        let pid = e.new_object();
        let tv = e.str_val(ty);
        let vv = e.str_val(val);
        e.set_own(pid, "type", tv, true);
        e.set_own(pid, "value", vv, true);
        out.push(Value::Obj(pid));
    }
    Ok(e.new_array(out))
}

fn dtf_range(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    // Approximation : `formatRange(a,b)` = `format(a)` (les formats
    // de plage requièrent des données CLDR de collapsage).
    dtf_format(e, this, args)
}

fn dtf_resolved(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let id = self_id(e, this, args, "DateTimeFormat")?;
    let opts = self_opts(e, id);
    let o = e.new_object();
    let loc = e.str_val("en-US");
    let cal = e.str_val("gregory");
    let ns = e.str_val("latn");
    let tz = e.str_val("UTC");
    let h12 = e.str_val(
        if opt(&opts, "hour12").is_none_or(|v| v != "false" && v != "24") {
            "h12"
        } else {
            "h23"
        },
    );
    e.set_own(o, "locale", loc, true);
    e.set_own(o, "calendar", cal, true);
    e.set_own(o, "numberingSystem", ns, true);
    e.set_own(o, "timeZone", tz, true);
    e.set_own(o, "hourCycle", h12, true);
    e.set_own(
        o,
        "hour12",
        Value::Bool(opt(&opts, "hour12").is_none_or(|v| v != "false" && v != "24")),
        true,
    );
    for k in [
        "dateStyle",
        "timeStyle",
        "weekday",
        "year",
        "month",
        "day",
        "hour",
        "minute",
        "second",
        "timeZoneName",
        "era",
        "dayPeriod",
    ] {
        if let Some(v) = opt(&opts, k) {
            let sv = e.str_val(v.to_string());
            e.set_own(o, k, sv, true);
        }
    }
    Ok(Value::Obj(o))
}

// -- NumberFormat ------------------------------------------------------

fn intl_nf_ctor(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let opts = read_opts(
        e,
        arg(args, 1),
        &[
            "style",
            "currency",
            "currencyDisplay",
            "currencySign",
            "unit",
            "unitDisplay",
            "minimumIntegerDigits",
            "minimumFractionDigits",
            "maximumFractionDigits",
            "minimumSignificantDigits",
            "maximumSignificantDigits",
            "useGrouping",
            "notation",
            "compactDisplay",
            "signDisplay",
            "roundingMode",
        ],
    )?;
    Ok(intl_obj(
        e,
        "NumberFormat",
        &opts,
        &[
            ("format", nf_format),
            ("formatToParts", nf_parts),
            ("formatRange", nf_range),
            ("resolvedOptions", nf_resolved),
        ],
    ))
}

/// Symbole monétaire en-US pour `code` (`$`, `€`, `£`, `¥`, sinon
/// le code suivi d'un espace — fallback dégradé).
fn currency_symbol(code: &str) -> &str {
    match code {
        "USD" | "CAD" | "AUD" | "NZD" | "HKD" | "SGD" | "MXN" => "$",
        "EUR" => "€",
        "GBP" => "£",
        "JPY" | "CNY" => "¥",
        "CHF" => "CHF ",
        "KRW" => "₩",
        "INR" => "₹",
        "RUB" => "₽",
        "BRL" => "R$",
        "BTC" => "₿",
        _ => "",
    }
}

/// `n` formaté : fraction bornée `[minf, maxf]`, groupement `,`,
/// minimum de chiffres entiers `mini`. Retourne `(int, frac)` texte.
fn fmt_decimal(n: f64, mini: usize, minf: usize, maxf: usize) -> (String, String) {
    // Notation scientifique Rust hors propos pour |n| < 1e21.
    let raw = if n.abs() < 1e21 {
        format!("{:.*}", maxf, n.abs())
    } else {
        format!("{:e}", n.abs())
    };
    let (int_s, frac_s) = match raw.split_once('.') {
        Some((i, f)) => (i.to_string(), f.to_string()),
        None => (raw.clone(), String::new()),
    };
    // Trim les zéros en trop au-delà de `minf`.
    let mut frac = frac_s;
    while frac.len() > minf && frac.ends_with('0') {
        frac.pop();
    }
    while frac.len() < minf {
        frac.push('0');
    }
    let mut int = int_s;
    while int.len() < mini {
        int.insert(0, '0');
    }
    (int, frac)
}

/// Insère les séparateurs de milliers `,` dans `int`.
fn group_int(int: &str) -> String {
    let bytes = int.as_bytes();
    let mut out = String::with_capacity(int.len() + int.len() / 3);
    for (i, b) in bytes.iter().enumerate() {
        if i > 0 && (bytes.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(*b as char);
    }
    out
}

/// Résout `(mini, minf, maxf)` pour `style`.
fn nf_digits(opts: &str) -> (usize, usize, usize) {
    let style = opt(opts, "style").unwrap_or("decimal");
    let (def_minf, def_maxf) = match style {
        "currency" => (2, 2),
        "percent" => (0, 0),
        _ => (0, 3),
    };
    let mini = opt(opts, "minimumIntegerDigits")
        .and_then(|v| v.parse().ok())
        .unwrap_or(1)
        .min(21);
    let minf = opt(opts, "minimumFractionDigits")
        .and_then(|v| v.parse().ok())
        .unwrap_or(def_minf)
        .min(100);
    let maxf = opt(opts, "maximumFractionDigits")
        .and_then(|v| v.parse().ok())
        .unwrap_or(def_maxf.max(minf))
        .max(minf)
        .min(100);
    (mini, minf, maxf)
}

/// Pièces `(type, valeur)` du formatage nombre.
fn nf_build(e: &mut Engine, opts: &str, n: f64) -> Vec<(String, String)> {
    let _ = e;
    let mut p: Vec<(String, String)> = Vec::new();
    let neg = n.is_sign_negative() && n != 0.0 || (n == 0.0 && n.is_sign_negative());
    let sign = opt(opts, "signDisplay").unwrap_or("auto");
    let show_sign = match sign {
        "always" => true,
        "exceptZero" => n != 0.0,
        "never" => false,
        _ => neg,
    };
    if show_sign {
        p.push(("sign".into(), if neg { "-".into() } else { "+".into() }));
    }
    if n.is_nan() {
        p.push(("nan".into(), "NaN".into()));
        return p;
    }
    if n.is_infinite() {
        p.push(("infinity".into(), "∞".into()));
        return p;
    }
    let style = opt(opts, "style").unwrap_or("decimal");
    let notation = opt(opts, "notation").unwrap_or("standard");
    let (mini, minf, maxf) = nf_digits(opts);
    let unit = opt(opts, "unit").unwrap_or("");
    let unit_disp = opt(opts, "unitDisplay").unwrap_or("short");

    let (mant, suffix_parts): (f64, Vec<(String, String)>) = match style {
        "percent" => {
            let m = n * 100.0;
            (m, vec![("percentSign".into(), "%".into())])
        }
        "currency" => {
            let code = opt(opts, "currency").unwrap_or("USD");
            let disp = opt(opts, "currencyDisplay").unwrap_or("symbol");
            match disp {
                "name" => {
                    let name = match code {
                        "USD" => "US dollars",
                        "EUR" => "euros",
                        "GBP" => "British pounds",
                        "JPY" => "Japanese yen",
                        _ => code,
                    };
                    (
                        n,
                        vec![
                            ("literal".into(), " ".into()),
                            ("currency".into(), name.into()),
                        ],
                    )
                }
                "code" => {
                    // en-US : code ISO en préfixe (`USD 5.00`).
                    p.push(("currency".into(), code.to_string()));
                    p.push(("literal".into(), " ".into()));
                    (n, Vec::new())
                }
                _ => {
                    let sym = currency_symbol(code);
                    if sym.is_empty() {
                        (n, vec![("currency".into(), format!("{code} "))])
                    } else {
                        // Préfixe `$`/`€`… poussé avant l'entier.
                        p.push(("currency".into(), sym.trim_end().to_string()));
                        (n, Vec::new())
                    }
                }
            }
        }
        _ => (n, Vec::new()),
    };

    // Notations compacte/scientifique (arrondi simple, pas de CLDR).
    let mut mant = mant;
    let mut notation_suffix = String::new();
    if notation == "compact" {
        let (div, suf) = if mant.abs() >= 1e12 {
            (1e12, "T")
        } else if mant.abs() >= 1e9 {
            (1e9, "B")
        } else if mant.abs() >= 1e6 {
            (1e6, "M")
        } else if mant.abs() >= 1e3 {
            (1e3, "K")
        } else {
            (1.0, "")
        };
        mant /= div;
        let (i, f) = fmt_decimal(mant, mini, minf, maxf.min(1));
        let mut s = i;
        if !f.is_empty() {
            s.push('.');
            s.push_str(&f);
        }
        while s.ends_with('0') && s.contains('.') {
            s.pop();
        }
        if s.ends_with('.') {
            s.pop();
        }
        p.push(("integer".into(), s));
        if !suf.is_empty() {
            notation_suffix = suf.to_string();
            p.push(("compact".into(), suf.into()));
        }
    } else if notation == "scientific" || notation == "engineering" {
        let exp = mant.abs().log10().floor() as i32;
        let exp = if notation == "engineering" {
            exp - exp.rem_euclid(3)
        } else {
            exp
        };
        let m2 = mant / 10f64.powi(exp);
        let (i, f) = fmt_decimal(m2, mini, minf, maxf);
        p.push(("integer".into(), i));
        if !f.is_empty() {
            p.push(("decimal".into(), ".".into()));
            p.push(("fraction".into(), f));
        }
        p.push(("exponentSeparator".into(), "E".into()));
        p.push(("exponentInteger".into(), exp.to_string()));
    } else {
        let (i, f) = fmt_decimal(mant, mini, minf, maxf);
        let grouped = match opt(opts, "useGrouping") {
            Some("false") => i,
            // `min2` : pas de séparateur sous 5 chiffres (spec).
            Some("min2") if i.len() <= 4 => i,
            _ => group_int(&i),
        };
        // Découpe integer/group pour formatToParts.
        let mut seg = String::new();
        for ch in grouped.chars() {
            if ch == ',' {
                if !seg.is_empty() {
                    p.push(("integer".into(), std::mem::take(&mut seg)));
                }
                p.push(("group".into(), ",".into()));
            } else {
                seg.push(ch);
            }
        }
        if !seg.is_empty() {
            p.push(("integer".into(), seg));
        }
        if !f.is_empty() {
            p.push(("decimal".into(), ".".into()));
            p.push(("fraction".into(), f));
        }
    }
    let _ = notation_suffix;

    if style == "unit" && !unit.is_empty() {
        let disp_name = if unit_disp == "long" {
            match unit {
                "meter" => " meters",
                "foot" | "feet" => " feet",
                "mile" => " miles",
                "kilometer" => " kilometers",
                "gram" => " grams",
                "kilogram" => " kilograms",
                "liter" => " liters",
                "celsius" => " degrees Celsius",
                "fahrenheit" => " degrees Fahrenheit",
                _ => "",
            }
        } else {
            match unit {
                "meter" => " m",
                "foot" | "feet" => " ft",
                "mile" => " mi",
                "kilometer" => " km",
                "gram" => " g",
                "kilogram" => " kg",
                "liter" => " L",
                "celsius" => "°C",
                "fahrenheit" => "°F",
                _ => "",
            }
        };
        if disp_name.is_empty() {
            p.push(("literal".into(), " ".into()));
            p.push(("unit".into(), unit.to_string()));
        } else {
            p.push(("unit".into(), disp_name.into()));
        }
    }
    p.extend(suffix_parts);
    p
}

fn nf_num(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<(ObjId, String, f64)> {
    let id = self_id(e, this, args, "NumberFormat")?;
    let opts = self_opts(e, id);
    let n = e.to_number(arg(args, 1))?;
    Ok((id, opts, n))
}

fn nf_format(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let (_id, opts, n) = nf_num(e, this, args)?;
    let parts = nf_build(e, &opts, n);
    Ok(e.str_val(parts.iter().map(|(_, v)| v.as_str()).collect::<String>()))
}

fn nf_parts(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let (_id, opts, n) = nf_num(e, this, args)?;
    let parts = nf_build(e, &opts, n);
    let mut out = Vec::with_capacity(parts.len());
    for (ty, val) in parts {
        let pid = e.new_object();
        let tv = e.str_val(ty);
        let vv = e.str_val(val);
        e.set_own(pid, "type", tv, true);
        e.set_own(pid, "value", vv, true);
        out.push(Value::Obj(pid));
    }
    Ok(e.new_array(out))
}

fn nf_range(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    nf_format(e, this, args)
}

fn nf_resolved(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let id = self_id(e, this, args, "NumberFormat")?;
    let opts = self_opts(e, id);
    let o = e.new_object();
    let loc = e.str_val("en-US");
    let ns = e.str_val("latn");
    let style = e.str_val(opt(&opts, "style").unwrap_or("decimal").to_string());
    e.set_own(o, "locale", loc, true);
    e.set_own(o, "numberingSystem", ns, true);
    e.set_own(o, "style", style, true);
    for k in [
        "currency",
        "currencyDisplay",
        "unit",
        "unitDisplay",
        "minimumIntegerDigits",
        "minimumFractionDigits",
        "maximumFractionDigits",
        "useGrouping",
        "notation",
        "signDisplay",
    ] {
        if let Some(v) = opt(&opts, k) {
            let sv = e.str_val(v.to_string());
            e.set_own(o, k, sv, true);
        }
    }
    Ok(Value::Obj(o))
}

// -- Collator ----------------------------------------------------------

fn intl_collator_ctor(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let opts = read_opts(
        e,
        arg(args, 1),
        &[
            "usage",
            "sensitivity",
            "ignorePunctuation",
            "numeric",
            "caseFirst",
            "localeMatcher",
            "collation",
        ],
    )?;
    Ok(intl_obj(
        e,
        "Collator",
        &opts,
        &[
            ("compare", collator_compare),
            ("resolvedOptions", collator_resolved),
        ],
    ))
}

fn collator_compare(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let id = self_id(e, this, args, "Collator")?;
    let opts = self_opts(e, id);
    let a = e.to_js_string(arg(args, 1))?;
    let b = e.to_js_string(arg(args, 2))?;
    let sensitivity = opt(&opts, "sensitivity").unwrap_or("variant");
    let numeric = opt(&opts, "numeric") == Some("true");
    let ign_punct = opt(&opts, "ignorePunctuation") == Some("true");
    let fold = |s: &str| -> String {
        let mut t: String = s
            .chars()
            .filter(|c| !(ign_punct && c.is_ascii_punctuation()))
            .collect();
        if matches!(sensitivity, "base" | "accent") {
            t = t.to_lowercase();
        }
        t
    };
    let (x, y) = (fold(&a), fold(&b));
    let ord = if numeric {
        let mut xi = x.chars().peekable();
        let mut yi = y.chars().peekable();
        loop {
            match (xi.peek().copied(), yi.peek().copied()) {
                (None, None) => break std::cmp::Ordering::Equal,
                (None, Some(_)) => break std::cmp::Ordering::Less,
                (Some(_), None) => break std::cmp::Ordering::Greater,
                (Some(c), Some(d)) if c.is_ascii_digit() && d.is_ascii_digit() => {
                    let mut ns = String::new();
                    let mut ms = String::new();
                    while xi.peek().is_some_and(char::is_ascii_digit) {
                        ns.push(xi.next().unwrap_or_default());
                    }
                    while yi.peek().is_some_and(char::is_ascii_digit) {
                        ms.push(yi.next().unwrap_or_default());
                    }
                    let (nx, mx) = (ns.trim_start_matches('0'), ms.trim_start_matches('0'));
                    let cmp = nx.len().cmp(&mx.len()).then_with(|| nx.cmp(mx));
                    if cmp != std::cmp::Ordering::Equal {
                        break cmp;
                    }
                }
                (Some(c), Some(d)) => {
                    if c != d {
                        break c.cmp(&d);
                    }
                    xi.next();
                    yi.next();
                }
            }
        }
    } else {
        x.cmp(&y)
    };
    Ok(Value::Num(match ord {
        std::cmp::Ordering::Less => -1.0,
        std::cmp::Ordering::Equal => 0.0,
        std::cmp::Ordering::Greater => 1.0,
    }))
}

fn collator_resolved(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let id = self_id(e, this, args, "Collator")?;
    let opts = self_opts(e, id);
    let o = e.new_object();
    let loc = e.str_val("en-US");
    let usage = e.str_val(opt(&opts, "usage").unwrap_or("sort").to_string());
    let sens = e.str_val(opt(&opts, "sensitivity").unwrap_or("variant").to_string());
    e.set_own(o, "locale", loc, true);
    e.set_own(o, "usage", usage, true);
    e.set_own(o, "sensitivity", sens, true);
    e.set_own(
        o,
        "ignorePunctuation",
        Value::Bool(opt(&opts, "ignorePunctuation") == Some("true")),
        true,
    );
    e.set_own(
        o,
        "numeric",
        Value::Bool(opt(&opts, "numeric") == Some("true")),
        true,
    );
    let cf = e.str_val("false");
    let col = e.str_val("default");
    e.set_own(o, "caseFirst", cf, true);
    e.set_own(o, "collation", col, true);
    Ok(Value::Obj(o))
}

// -- PluralRules -------------------------------------------------------

fn intl_plural_ctor(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let opts = read_opts(
        e,
        arg(args, 1),
        &[
            "type",
            "minimumIntegerDigits",
            "minimumFractionDigits",
            "maximumFractionDigits",
        ],
    )?;
    Ok(intl_obj(
        e,
        "PluralRules",
        &opts,
        &[
            ("select", plural_select),
            ("selectRange", plural_range),
            ("resolvedOptions", plural_resolved),
        ],
    ))
}

fn plural_select(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let id = self_id(e, this, args, "PluralRules")?;
    let opts = self_opts(e, id);
    let n = e.to_number(arg(args, 1))?;
    // Règles `en` : cardinal — `one` si entier == 1 sans fraction
    // visible ; ordinal — 1st/2nd/3rd/…th.
    let cat = if opt(&opts, "type") == Some("ordinal") {
        let i = n.abs() as i64;
        match (i % 10, i % 100) {
            (1, r) if r != 11 => "one",
            (2, r) if r != 12 => "two",
            (3, r) if r != 13 => "few",
            _ => "other",
        }
    } else if n.fract().abs() < f64::EPSILON && (n.abs() - 1.0).abs() < f64::EPSILON {
        "one"
    } else {
        "other"
    };
    Ok(e.str_val(cat))
}

fn plural_range(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let _ = self_id(e, this, args, "PluralRules")?;
    // Pas de données CLDR de plage — résultat le plus fréquent.
    Ok(e.str_val("other"))
}

fn plural_resolved(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let id = self_id(e, this, args, "PluralRules")?;
    let opts = self_opts(e, id);
    let o = e.new_object();
    let loc = e.str_val("en-US");
    let ty = e.str_val(opt(&opts, "type").unwrap_or("cardinal").to_string());
    e.set_own(o, "locale", loc, true);
    e.set_own(o, "type", ty, true);
    let names: &[&str] = if opt(&opts, "type") == Some("ordinal") {
        &["one", "two", "few", "other"]
    } else {
        &["one", "other"]
    };
    let cats: Vec<Value> = names.iter().map(|c| e.str_val(*c)).collect();
    let arr = e.new_array(cats);
    e.set_own(o, "pluralCategories", arr, true);
    Ok(Value::Obj(o))
}

// -- ListFormat --------------------------------------------------------

fn intl_list_ctor(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let opts = read_opts(e, arg(args, 1), &["type", "style"])?;
    Ok(intl_obj(
        e,
        "ListFormat",
        &opts,
        &[
            ("format", list_format),
            ("formatToParts", list_parts),
            ("resolvedOptions", list_resolved),
        ],
    ))
}

/// Éléments chaîne d'une liste JS (`undefined` → vide, spec).
fn list_items(e: &mut Engine, v: Value) -> JsResult<Vec<String>> {
    if matches!(v, Value::Undefined) {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    for item in e.iter_values(v)? {
        out.push(e.to_js_string(item)?);
    }
    Ok(out)
}

fn list_join(e: &mut Engine, opts: &str, items: &[String]) -> Vec<(String, String)> {
    let _ = e;
    let ty = opt(opts, "type").unwrap_or("conjunction");
    let style = opt(opts, "style").unwrap_or("long");
    let conj = match (ty, style) {
        ("disjunction", _) => "or",
        ("unit", _) => "",
        ("conjunction", "narrow" | "short") => "&",
        _ => "and",
    };
    let sep = if ty == "unit" && style == "narrow" {
        " "
    } else {
        ", "
    };
    let mut p: Vec<(String, String)> = Vec::new();
    let n = items.len();
    for (i, s) in items.iter().enumerate() {
        p.push(("element".into(), s.clone()));
        if i + 1 == n {
            break;
        }
        if n == 2 {
            p.push((
                "literal".into(),
                if conj.is_empty() {
                    sep.into()
                } else {
                    format!(" {conj} ")
                },
            ));
        } else if i + 2 == n {
            // Dernier lien : ", and x" (en-US garde la virgule Oxford).
            if conj.is_empty() {
                p.push(("literal".into(), sep.into()));
            } else {
                p.push(("literal".into(), format!(", {conj} ")));
            }
        } else {
            p.push(("literal".into(), sep.into()));
        }
    }
    p
}

fn list_format(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let id = self_id(e, this, args, "ListFormat")?;
    let opts = self_opts(e, id);
    let items = list_items(e, arg(args, 1))?;
    let parts = list_join(e, &opts, &items);
    Ok(e.str_val(parts.iter().map(|(_, v)| v.as_str()).collect::<String>()))
}

fn list_parts(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let id = self_id(e, this, args, "ListFormat")?;
    let opts = self_opts(e, id);
    let items = list_items(e, arg(args, 1))?;
    let parts = list_join(e, &opts, &items);
    let mut out = Vec::with_capacity(parts.len());
    for (ty, val) in parts {
        let pid = e.new_object();
        let tv = e.str_val(ty);
        let vv = e.str_val(val);
        e.set_own(pid, "type", tv, true);
        e.set_own(pid, "value", vv, true);
        out.push(Value::Obj(pid));
    }
    Ok(e.new_array(out))
}

fn list_resolved(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let id = self_id(e, this, args, "ListFormat")?;
    let opts = self_opts(e, id);
    let o = e.new_object();
    let loc = e.str_val("en-US");
    let ty = e.str_val(opt(&opts, "type").unwrap_or("conjunction").to_string());
    let st = e.str_val(opt(&opts, "style").unwrap_or("long").to_string());
    e.set_own(o, "locale", loc, true);
    e.set_own(o, "type", ty, true);
    e.set_own(o, "style", st, true);
    Ok(Value::Obj(o))
}

// -- RelativeTimeFormat -------------------------------------------------

fn intl_rtf_ctor(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let opts = read_opts(e, arg(args, 1), &["style", "numeric"])?;
    Ok(intl_obj(
        e,
        "RelativeTimeFormat",
        &opts,
        &[
            ("format", rtf_format),
            ("formatToParts", rtf_parts),
            ("resolvedOptions", rtf_resolved),
        ],
    ))
}

/// Parties `in N days` / `N days ago` selon `unit`/`style`/`numeric`.
fn rtf_build(e: &mut Engine, opts: &str, n: f64, unit: &str) -> Vec<(String, String)> {
    let _ = e;
    let style = opt(opts, "style").unwrap_or("long");
    let numeric = opt(opts, "numeric").unwrap_or("always");
    let u = unit.trim_end_matches('s');
    let (one, many) = match (u, style) {
        ("second", "long") => ("second", "seconds"),
        ("second", _) => ("sec.", "sec."),
        ("minute", "long") => ("minute", "minutes"),
        ("minute", _) => ("min.", "min."),
        ("hour", "long") => ("hour", "hours"),
        ("hour", _) => ("hr.", "hr."),
        ("day", _) => ("day", "days"),
        ("week", "long") => ("week", "weeks"),
        ("week", _) => ("wk.", "wk."),
        ("month", "long") => ("month", "months"),
        ("month", _) => ("mo.", "mo."),
        ("quarter", "long") => ("quarter", "quarters"),
        ("quarter", _) => ("qtr.", "qtr."),
        ("year", "long") => ("year", "years"),
        ("year", _) => ("yr.", "yr."),
        _ => (u, u),
    };
    // `numeric: auto` — formes idiomatiques `en` (day/second).
    if numeric == "auto" && style == "long" {
        let word = match (u, n) {
            ("day", -1.0) => Some("yesterday"),
            ("day", 0.0) => Some("today"),
            ("day", 1.0) => Some("tomorrow"),
            ("week", -1.0) => Some("last week"),
            ("week", 0.0) => Some("this week"),
            ("week", 1.0) => Some("next week"),
            ("month", -1.0) => Some("last month"),
            ("month", 0.0) => Some("this month"),
            ("month", 1.0) => Some("next month"),
            ("year", -1.0) => Some("last year"),
            ("year", 0.0) => Some("this year"),
            ("year", 1.0) => Some("next year"),
            ("second", 0.0) => Some("now"),
            _ => None,
        };
        if let Some(w) = word {
            return vec![("literal".into(), w.to_string())];
        }
    }
    let num = (n.abs()).to_string();
    let unit_txt = if (n.abs() - 1.0).abs() < f64::EPSILON {
        one
    } else {
        many
    };
    if n < 0.0 {
        vec![
            ("integer".into(), num),
            ("literal".into(), format!(" {unit_txt} ago")),
        ]
    } else {
        vec![
            ("literal".into(), "in ".into()),
            ("integer".into(), num),
            ("literal".into(), format!(" {unit_txt}")),
        ]
    }
}

fn rtf_format(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let id = self_id(e, this, args, "RelativeTimeFormat")?;
    let opts = self_opts(e, id);
    let n = e.to_number(arg(args, 1))?;
    let unit = e.to_js_string(arg(args, 2))?;
    let parts = rtf_build(e, &opts, n, &unit);
    Ok(e.str_val(parts.iter().map(|(_, v)| v.as_str()).collect::<String>()))
}

fn rtf_parts(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let id = self_id(e, this, args, "RelativeTimeFormat")?;
    let opts = self_opts(e, id);
    let n = e.to_number(arg(args, 1))?;
    let unit = e.to_js_string(arg(args, 2))?;
    let parts = rtf_build(e, &opts, n, &unit);
    let mut out = Vec::with_capacity(parts.len());
    for (ty, val) in parts {
        let pid = e.new_object();
        let tv = e.str_val(ty);
        let vv = e.str_val(val);
        e.set_own(pid, "type", tv, true);
        e.set_own(pid, "value", vv, true);
        out.push(Value::Obj(pid));
    }
    Ok(e.new_array(out))
}

fn rtf_resolved(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let id = self_id(e, this, args, "RelativeTimeFormat")?;
    let opts = self_opts(e, id);
    let o = e.new_object();
    let loc = e.str_val("en-US");
    let st = e.str_val(opt(&opts, "style").unwrap_or("long").to_string());
    let nu = e.str_val(opt(&opts, "numeric").unwrap_or("always").to_string());
    e.set_own(o, "locale", loc, true);
    e.set_own(o, "style", st, true);
    let nsn = e.str_val("latn");
    e.set_own(o, "numeric", nu, true);
    e.set_own(o, "numberingSystem", nsn, true);
    Ok(Value::Obj(o))
}

// -- Locale ------------------------------------------------------------

fn intl_locale_ctor(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let tag = e.to_js_string(arg(args, 0))?;
    let canon = canonical_tag(&tag)
        .filter(|c| !c.is_empty())
        .unwrap_or_else(|| "en-US".to_string());
    let mut opts = String::from("tag=");
    opts.push_str(&canon);
    opts.push('\u{1}');
    // Options override du tag (`{region: 'FR'}` → `de-AT`).
    let (mut lang, mut script, mut region) = locale_parts(&canon);
    if let Value::Obj(oid) = arg(args, 1) {
        for k in [
            "region",
            "script",
            "language",
            "calendar",
            "numberingSystem",
        ] {
            if let Some(v) = e.get_prop(oid, k) {
                if matches!(v, Value::Undefined) {
                    continue;
                }
                let sv = e.to_js_string(v)?;
                match k {
                    "language" => lang = sv.to_ascii_lowercase(),
                    "script" => {
                        script = {
                            let mut c = sv.chars();
                            format!(
                                "{}{}",
                                c.next().unwrap_or_default().to_ascii_uppercase(),
                                c.as_str().to_ascii_lowercase()
                            )
                        }
                    }
                    "region" => region = sv.to_ascii_uppercase(),
                    _ => {}
                }
                opts.push_str(k);
                opts.push('=');
                opts.push_str(&sv);
                opts.push('\u{1}');
            }
        }
    }
    // Tag final = tag canonique remplacé par les overrides.
    let mut final_tag = lang;
    if !script.is_empty() {
        final_tag.push('-');
        final_tag.push_str(&script);
    }
    if !region.is_empty() {
        final_tag.push('-');
        final_tag.push_str(&region);
    }
    opts = format!("tag={final_tag}\u{1}{opts}");
    Ok(intl_obj(
        e,
        "Locale",
        &opts,
        &[
            ("toString", locale_to_string),
            ("maximize", locale_maximize),
            ("minimize", locale_minimize),
        ],
    ))
}

/// `(language, script, region)` du tag canonique.
fn locale_parts(tag: &str) -> (String, String, String) {
    let mut lang = String::new();
    let mut script = String::new();
    let mut region = String::new();
    for (i, sub) in tag.split('-').enumerate() {
        if i == 0 {
            lang = sub.to_string();
        } else if sub.len() == 4 && sub.chars().all(|c| c.is_ascii_alphabetic()) {
            script = sub.to_string();
        } else if sub.len() == 2 || (sub.len() == 3 && sub.chars().all(|c| c.is_ascii_digit())) {
            region = sub.to_string();
        }
    }
    (lang, script, region)
}

fn locale_to_string(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let id = self_id(e, this, args, "Locale")?;
    let opts = self_opts(e, id);
    Ok(e.str_val(opt(&opts, "tag").unwrap_or("en-US").to_string()))
}

fn locale_maximize(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    // Sans données de likely-subtags, `maximize()` rend une copie.
    let id = self_id(e, this, args, "Locale")?;
    let opts = self_opts(e, id);
    let tag = opt(&opts, "tag").unwrap_or("en-US").to_string();
    Ok(intl_obj(
        e,
        "Locale",
        &format!("tag={tag}\u{1}"),
        &[
            ("toString", locale_to_string),
            ("maximize", locale_maximize),
            ("minimize", locale_minimize),
        ],
    ))
}

fn locale_minimize(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let id = self_id(e, this, args, "Locale")?;
    let opts = self_opts(e, id);
    let tag = opt(&opts, "tag").unwrap_or("en-US");
    let (lang, _, _) = locale_parts(tag);
    Ok(intl_obj(
        e,
        "Locale",
        &format!("tag={lang}\u{1}"),
        &[
            ("toString", locale_to_string),
            ("maximize", locale_maximize),
            ("minimize", locale_minimize),
        ],
    ))
}

// -- DisplayNames -------------------------------------------------------

fn intl_display_ctor(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let opts = read_opts(e, arg(args, 1), &["type", "style", "fallback"])?;
    Ok(intl_obj(
        e,
        "DisplayNames",
        &opts,
        &[("of", display_of), ("resolvedOptions", display_resolved)],
    ))
}

/// Nom d'affichage `en` pour `code` selon `type` ; repli sur le
/// code lui-même (spec `fallback: 'code'`).
fn display_name(ty: &str, code: &str) -> &'static str {
    match ty {
        "language" => match code.to_ascii_lowercase().as_str() {
            "en" => "English",
            "fr" => "French",
            "de" => "German",
            "es" => "Spanish",
            "it" => "Italian",
            "pt" => "Portuguese",
            "ja" => "Japanese",
            "zh" => "Chinese",
            "ko" => "Korean",
            "ru" => "Russian",
            "ar" => "Arabic",
            "hi" => "Hindi",
            "nl" => "Dutch",
            "pl" => "Polish",
            "tr" => "Turkish",
            "sv" => "Swedish",
            "uk" => "Ukrainian",
            _ => "",
        },
        "region" => match code.to_ascii_uppercase().as_str() {
            "US" => "United States",
            "GB" | "UK" => "United Kingdom",
            "FR" => "France",
            "DE" => "Germany",
            "ES" => "Spain",
            "IT" => "Italy",
            "PT" => "Portugal",
            "JP" => "Japan",
            "CN" => "China",
            "KR" => "South Korea",
            "RU" => "Russia",
            "BR" => "Brazil",
            "CA" => "Canada",
            "MX" => "Mexico",
            "IN" => "India",
            "AU" => "Australia",
            "NL" => "Netherlands",
            "CH" => "Switzerland",
            "BE" => "Belgium",
            "SE" => "Sweden",
            "NO" => "Norway",
            "DK" => "Denmark",
            "FI" => "Finland",
            "PL" => "Poland",
            "TR" => "Türkiye",
            "UA" => "Ukraine",
            "AR" => "Argentina",
            "ZA" => "South Africa",
            "EU" => "European Union",
            "001" => "world",
            _ => "",
        },
        "script" => match code {
            "Latn" => "Latin",
            "Cyrl" => "Cyrillic",
            "Arab" => "Arabic",
            "Hans" => "Simplified",
            "Hant" => "Traditional",
            "Jpan" => "Japanese",
            "Kore" => "Korean",
            "Deva" => "Devanagari",
            "Grek" => "Greek",
            "Hebr" => "Hebrew",
            "Thai" => "Thai",
            _ => "",
        },
        "currency" => match code.to_ascii_uppercase().as_str() {
            "USD" => "US Dollar",
            "EUR" => "Euro",
            "GBP" => "British Pound",
            "JPY" => "Japanese Yen",
            "CHF" => "Swiss Franc",
            "CAD" => "Canadian Dollar",
            "AUD" => "Australian Dollar",
            "CNY" => "Chinese Yuan",
            "KRW" => "South Korean Won",
            "INR" => "Indian Rupee",
            "BRL" => "Brazilian Real",
            "MXN" => "Mexican Peso",
            "RUB" => "Russian Ruble",
            _ => "",
        },
        _ => "",
    }
}

fn display_of(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let id = self_id(e, this, args, "DisplayNames")?;
    let opts = self_opts(e, id);
    let ty = opt(&opts, "type").unwrap_or("language").to_string();
    let code = e.to_js_string(arg(args, 1))?;
    let name = display_name(&ty, &code);
    if name.is_empty() {
        return Ok(e.str_val(code));
    }
    Ok(e.str_val(name.to_string()))
}

fn display_resolved(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let id = self_id(e, this, args, "DisplayNames")?;
    let opts = self_opts(e, id);
    let o = e.new_object();
    let loc = e.str_val("en-US");
    let ty = e.str_val(opt(&opts, "type").unwrap_or("language").to_string());
    let st = e.str_val(opt(&opts, "style").unwrap_or("long").to_string());
    let fb = e.str_val(opt(&opts, "fallback").unwrap_or("code").to_string());
    e.set_own(o, "locale", loc, true);
    e.set_own(o, "type", ty, true);
    e.set_own(o, "style", st, true);
    e.set_own(o, "fallback", fb, true);
    Ok(Value::Obj(o))
}

// -- accesseurs `Locale` sur le proto ----------------------------------

/// Installe les getters `Locale` sur le proto (`baseName`,
/// `language`, `script`, `region`, `calendar`, `numberingSystem`).
fn install_locale_accessors(e: &mut Engine) {
    let Some(Value::Obj(intl_id)) = e.get_prop(e.global, "Intl") else {
        return;
    };
    let Some(Value::Obj(cid)) = e.get_prop(intl_id, "Locale") else {
        return;
    };
    let Some(Value::Obj(pid)) = e.get_prop(cid, "prototype") else {
        return;
    };
    for (name, f) in [
        ("baseName", locale_g_tag as crate::heap::NativeFn),
        ("language", locale_g_lang),
        ("script", locale_g_script),
        ("region", locale_g_region),
        ("calendar", locale_g_opt_cal),
        ("numberingSystem", locale_g_opt_ns),
    ] {
        let gv = e.native_fn("get", f);
        if let Some(o) = e.heap.obj_mut(pid) {
            o.set_accessor(name, gv, Value::Undefined, false);
        }
    }
}

/// Tag canonique de l'instance (`__opts` `tag=…`).
fn locale_tag_of(e: &mut Engine, this: Value) -> JsResult<String> {
    let id = self_id(e, this, &[], "Locale")?;
    let opts = self_opts(e, id);
    Ok(opt(&opts, "tag").unwrap_or("en-US").to_string())
}

fn locale_g_tag(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    let t = locale_tag_of(e, this)?;
    Ok(e.str_val(t))
}

fn locale_g_lang(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    let t = locale_tag_of(e, this)?;
    let (lang, _, _) = locale_parts(&t);
    Ok(e.str_val(lang))
}

fn locale_g_script(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    let t = locale_tag_of(e, this)?;
    let (_, script, _) = locale_parts(&t);
    if script.is_empty() {
        return Ok(Value::Undefined);
    }
    Ok(e.str_val(script))
}

fn locale_g_region(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    let t = locale_tag_of(e, this)?;
    let (_, _, region) = locale_parts(&t);
    if region.is_empty() {
        return Ok(Value::Undefined);
    }
    Ok(e.str_val(region))
}

fn locale_g_opt_cal(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    let id = self_id(e, this, &[], "Locale")?;
    let opts = self_opts(e, id);
    Ok(e.str_val(opt(&opts, "calendar").unwrap_or("gregory").to_string()))
}

fn locale_g_opt_ns(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    let id = self_id(e, this, &[], "Locale")?;
    let opts = self_opts(e, id);
    Ok(e.str_val(opt(&opts, "numberingSystem").unwrap_or("latn").to_string()))
}

// -- `Intl.Segmenter` (CAP-190, ES2022) — borné en-US -----------------
// Indices en unités UTF-16 (spec). Pas de CLDR : frontières
// approchées honnêtes — grapheme = paires surrogates +
// étendues (diacritiques, modifieurs emoji, tags, ZWJ,
// indicateurs régionaux par paires) ; word = transitions
// alphanumérique↔autre, `isWordLike` = segment alnum ;
// sentence = après [.!?]+ suivi de blanc/fin.

/// `new Intl.Segmenter(locales, {granularity})` — `word`/
/// `sentence`/`grapheme` (défaut) ; autre = `RangeError`.
fn intl_segmenter_ctor(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let opts = read_opts(e, arg(args, 1), &["granularity"])?;
    match opt(&opts, "granularity") {
        None | Some("grapheme" | "word" | "sentence") => {}
        Some(g) => return Err(e.range_error(format!("invalid granularity «{g}»"))),
    }
    Ok(intl_obj(
        e,
        "Segmenter",
        &opts,
        &[("segment", seg_segment), ("resolvedOptions", seg_resolved)],
    ))
}

/// `(segment_utf16, index_u16, isWordLike)` — découpe
/// `input` selon la granularité.
fn split_segments(input: &str, gran: &str) -> Vec<(String, usize, bool)> {
    let u: Vec<u16> = input.encode_utf16().collect();
    let n = u.len();
    let mut bounds: Vec<usize> = vec![0];
    match gran {
        "word" => {
            // Frontière à chaque transition alnum↔autre.
            let mut prev_w = false;
            let mut off = 0usize;
            for ch in input.chars() {
                let w = ch.is_alphanumeric() || ch == '_';
                if off > 0 && w != prev_w {
                    bounds.push(off);
                }
                prev_w = w;
                off += ch.len_utf16();
            }
        }
        "sentence" => {
            // Frontière au premier non-blanc après [.!?]+blancs*.
            let mut off = 0usize;
            let mut pend = false;
            for ch in input.chars() {
                if pend && !ch.is_whitespace() {
                    bounds.push(off);
                    pend = false;
                }
                if matches!(ch, '.' | '!' | '?') {
                    pend = true;
                }
                off += ch.len_utf16();
            }
        }
        _ => {
            // grapheme — sous-ensemble UAX #29.
            let mut i = 1usize;
            while i < n {
                if grapheme_break(&u, i) {
                    bounds.push(i);
                }
                i += 1;
            }
        }
    }
    if n == 0 {
        return Vec::new();
    }
    bounds.dedup();
    let mut out = Vec::with_capacity(bounds.len());
    for (k, &b) in bounds.iter().enumerate() {
        let e2 = bounds.get(k + 1).copied().unwrap_or(n);
        let s = String::from_utf16_lossy(&u[b..e2]);
        let wl = s.chars().any(char::is_alphanumeric);
        out.push((s, b, wl));
    }
    out
}

/// Pas de coupure : entre surrogates, avant étendue/ZWJ+1,
/// entre indicateurs régionaux appariés.
fn grapheme_break(u: &[u16], i: usize) -> bool {
    let prev = u[i - 1];
    let cur = u[i];
    if (0xD800..0xDC00).contains(&prev) && (0xDC00..0xE000).contains(&cur) {
        return false;
    }
    if is_extend(cur) || prev == 0x200D {
        return false;
    }
    // Indicateur régional (drapeaux) : paires — pas de
    // coupure si le RI courant complète une paire.
    let ri = |j: usize| j + 1 < u.len() && u[j] == 0xD83C && (0xDDE6..=0xDDFF).contains(&u[j + 1]);
    if ri(i) {
        let mut k = i;
        let mut cnt = 0usize;
        while k >= 2 && ri(k - 2) {
            cnt += 1;
            k -= 2;
        }
        if cnt % 2 == 1 {
            return false;
        }
    }
    true
}

/// Étendues UAX #29 (sous-ensemble) : marques combinaison,
/// modifieurs emoji, tags, ZWNJ.
fn is_extend(c: u16) -> bool {
    matches!(
        c,
        0x0300..=0x036F
            | 0x1AB0..=0x1AFF
            | 0x1DC0..=0x1DFF
            | 0x20D0..=0x20FF
            | 0xFE00..=0xFE0F
            | 0xFE20..=0xFE2F
            | 0x3099..=0x309A
            | 0x200C
    )
}

/// `seg.segment(input)` → objet `Segments` itérable :
/// `[Symbol.iterator]` + `containing(i)` + items
/// `{segment, index, isWordLike, input}`.
fn seg_segment(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let id = self_id(e, this, args, "Segmenter")?;
    let input = e.to_js_string(arg(args, 1))?;
    let gran = {
        let o = self_opts(e, id);
        opt(&o, "granularity").unwrap_or("grapheme").to_string()
    };
    let mut list = Vec::new();
    for (s, idx, wl) in split_segments(&input, &gran) {
        let o = e.new_object();
        let sv = e.str_val(s);
        e.set_own(o, "segment", sv, true);
        e.set_own(o, "index", Value::Num(idx as f64), true);
        e.set_own(o, "isWordLike", Value::Bool(wl), true);
        let iv = e.str_val(input.clone());
        e.set_own(o, "input", iv, true);
        list.push(Value::Obj(o));
    }
    let arr = e.new_array(list);
    let seg = e.new_object();
    e.set_own(seg, "__segs", arr, false);
    let iv = e.str_val(input);
    e.set_own(seg, "__input", iv, false);
    let seg_v = Value::Obj(seg);
    let it = e.bound_native("[Symbol.iterator]", segs_iter_new, vec![seg_v]);
    e.set_own(seg, crate::interp::SYM_ITERATOR, it, false);
    let cont = e.bound_native("containing", segs_containing, vec![seg_v]);
    e.set_own(seg, "containing", cont, true);
    Ok(seg_v)
}

/// `segments[Symbol.iterator]()` → itérateur frais
/// (`__i` curseur sur `__segs`).
fn segs_iter_new(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let list = match arg(args, 0) {
        Value::Obj(id) => e.get_prop(id, "__segs").unwrap_or(Value::Undefined),
        _ => Value::Undefined,
    };
    let it = e.new_object();
    e.set_own(it, "__i", Value::Num(0.0), false);
    e.set_own(it, "__segs", list, false);
    let it_v = Value::Obj(it);
    let next = e.bound_native("next", segs_iter_next, vec![it_v]);
    e.set_own(it, "next", next, true);
    Ok(it_v)
}

/// `iterator.next()` → `{value, done}` sur `__segs[__i]`.
fn segs_iter_next(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let r = e.new_object();
    if let Value::Obj(it) = arg(args, 0) {
        let i = match e.get_prop(it, "__i") {
            Some(Value::Num(n)) => n as usize,
            _ => 0,
        };
        let item = match e.get_prop(it, "__segs") {
            Some(Value::Obj(l)) => e.get_prop(l, &i.to_string()).unwrap_or(Value::Undefined),
            _ => Value::Undefined,
        };
        if matches!(item, Value::Undefined) {
            e.set_own(r, "value", Value::Undefined, true);
            e.set_own(r, "done", Value::Bool(true), true);
        } else {
            e.set_own(it, "__i", Value::Num((i + 1) as f64), false);
            e.set_own(r, "value", item, true);
            e.set_own(r, "done", Value::Bool(false), true);
        }
    } else {
        e.set_own(r, "value", Value::Undefined, true);
        e.set_own(r, "done", Value::Bool(true), true);
    }
    Ok(Value::Obj(r))
}

/// `segments.containing(i)` → item dont `index ≤ i < fin`.
fn segs_containing(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let (Value::Obj(segobj), Value::Num(n)) = (arg(args, 0), arg(args, 1)) else {
        return Ok(Value::Undefined);
    };
    let Some(Value::Obj(list)) = e.get_prop(segobj, "__segs") else {
        return Ok(Value::Undefined);
    };
    let len = match e.get_prop(list, "length") {
        Some(Value::Num(x)) => x as usize,
        _ => 0,
    };
    let n = n as usize;
    for i in 0..len {
        let Some(item @ Value::Obj(it)) = e.get_prop(list, &i.to_string()) else {
            continue;
        };
        let idx = match e.get_prop(it, "index") {
            Some(Value::Num(x)) => x as usize,
            _ => 0,
        };
        if idx > n {
            break;
        }
        let end = if i + 1 < len {
            match e.get_prop(list, &(i + 1).to_string()) {
                Some(Value::Obj(nx)) => match e.get_prop(nx, "index") {
                    Some(Value::Num(x)) => x as usize,
                    _ => usize::MAX,
                },
                _ => usize::MAX,
            }
        } else {
            usize::MAX
        };
        if n < end {
            return Ok(item);
        }
    }
    Ok(Value::Undefined)
}

/// `seg.resolvedOptions()` → `{locale:'en-US', granularity}`.
fn seg_resolved(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let id = self_id(e, this, args, "Segmenter")?;
    let opts = self_opts(e, id);
    let o = e.new_object();
    let loc = e.str_val("en-US");
    e.set_own(o, "locale", loc, true);
    let g = e.str_val(opt(&opts, "granularity").unwrap_or("grapheme"));
    e.set_own(o, "granularity", g, true);
    Ok(Value::Obj(o))
}
