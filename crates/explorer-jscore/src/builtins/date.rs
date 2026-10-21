// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Builtins JS — section extraite de `builtins.rs` (JS-C40, pur
//! reformatage mécanique : sémantique et tests inchangés).

#[allow(clippy::wildcard_imports)]
use super::*;

// ------------------------------------------------------------------
// JS-7g — full-ish UTC `Date`. Time zone is always UTC (the engine
// has no tz db; `getTimezoneOffset` = 0, `getUTC*` == `get*`).
// Civil ↔ days algorithms are Hinnant's proleptic-Gregorian ones.
// ------------------------------------------------------------------

/// Days since epoch for `y-m-d` (month is 1-based here).
pub(crate) fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (m + 9).rem_euclid(12);
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// `(y, m, d)` from days since epoch.
pub(crate) fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// `ms` since epoch → `(y, mo, d, weekday, h, mi, s, ms)`.
pub(crate) fn ms_to_parts(ms: f64) -> (i64, u32, u32, u32, u32, u32, u32, u32) {
    if !ms.is_finite() {
        return (0, 0, 0, 0, 0, 0, 0, 0);
    }
    let days = ms.div_euclid(86_400_000.0) as i64;
    let rem = ms.rem_euclid(86_400_000.0) as i64;
    let (y, mo, d) = civil_from_days(days);
    let wd = (days.rem_euclid(7) + 4).rem_euclid(7) as u32;
    let h = (rem / 3_600_000) as u32;
    let mi = (rem % 3_600_000 / 60_000) as u32;
    let s = (rem % 60_000 / 1000) as u32;
    let mss = (rem % 1000) as u32;
    (y, mo, d, wd, h, mi, s, mss)
}

/// `y, mo(0-based), d, h, mi, s, ms` → epoch ms (UTC).
pub(crate) fn parts_to_ms(y: i64, mo: i64, d: i64, h: i64, mi: i64, s: i64, mss: i64) -> f64 {
    // `y` < 100 maps to 1900+y (spec quirk kept for parity).
    let y = if (0..=99).contains(&y) { 1900 + y } else { y };
    (days_from_civil(y, mo + 1, d) * 86_400_000 + h * 3_600_000 + mi * 60_000 + s * 1000 + mss)
        as f64
}

/// Lenient ISO-8601 parse: `YYYY-MM-DD[THH:mm[:ss[.sss]]][Z|±hh:mm]`.
/// Returns epoch ms (UTC applied on `Z`/offset, treated as UTC
/// without one — we have no tz). Non-ISO → `NaN`.
pub(crate) fn parse_iso_date(s: &str) -> Option<f64> {
    let s = s.trim();
    // Un format ISO ne contient aucune lettre hors `T`/`Z` —
    // "15 Jan 2000" tombait ici et lisait `15` comme l'année 1915.
    if s.chars()
        .any(|c| c.is_ascii_alphabetic() && !matches!(c, 'T' | 't' | 'Z' | 'z'))
    {
        return None;
    }
    let (date, time) = s.split_once(['T', ' ']).map_or((s, ""), |(d, t)| (d, t));
    let mut dp = date.split('-');
    let y: i64 = dp.next()?.parse().ok()?;
    let mo: i64 = dp.next().map_or(1, |m| m.parse().unwrap_or(1));
    let d: i64 = dp.next().map_or(1, |d| d.parse().unwrap_or(1));
    if !(1..=12).contains(&mo) || !(1..=31).contains(&d) {
        return None;
    }
    let (mut h, mut mi, mut sec, mut mss) = (0i64, 0i64, 0i64, 0i64);
    let mut offset_min = 0i64;
    if !time.is_empty() {
        let (t, off) = if let Some(t) = time.strip_suffix('Z') {
            (t, 0i64)
        } else if let Some(i) = time.rfind(['+', '-']).filter(|&i| i > 0) {
            let (h_off, m_off) = time[i + 1..]
                .split_once(':')
                .map_or((time[i + 1..].parse().unwrap_or(0), 0), |(h, m)| {
                    (h.parse().unwrap_or(0), m.parse().unwrap_or(0))
                });
            let sign = if time.as_bytes()[i] == b'-' { -1 } else { 1 };
            (&time[..i], sign * (h_off * 60 + m_off))
        } else {
            (time, 0i64)
        };
        let mut tp = t.split(':');
        h = tp.next().map_or(0, |v| v.parse().unwrap_or(0));
        mi = tp.next().map_or(0, |v| v.parse().unwrap_or(0));
        if let Some(ss) = tp.next() {
            let (s_i, ms_f) = ss.split_once('.').map_or((ss, "0"), |(a, b)| (a, b));
            sec = s_i.parse().unwrap_or(0);
            mss = format!("{ms_f:0<3}")[..3].parse().unwrap_or(0);
        }
        offset_min = off;
    }
    Some(parts_to_ms(y, mo - 1, d, h, mi, sec, mss) - (offset_min * 60_000) as f64)
}

/// `Date.parse(str)` → epoch ms or `NaN`.
pub(crate) fn date_parse(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let s = e.to_js_string(arg(args, 0))?;
    Ok(Value::Num(parse_js_date(&s).unwrap_or(f64::NAN)))
}

/// `Date.UTC(y, mo, …)` → epoch ms.
pub(crate) fn date_utc(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let mut n = |i: usize| -> JsResult<i64> {
        Ok(match args.get(i) {
            Some(v) => e.to_number(*v)? as i64,
            None => 0,
        })
    };
    if args.is_empty() {
        return Ok(Value::Num(f64::NAN));
    }
    let y = n(0)?;
    let mo = n(1)?;
    let d = n(2)?.max(1);
    let (h, mi, s, ms) = (n(3)?, n(4)?, n(5)?, n(6)?);
    Ok(Value::Num(parts_to_ms(y, mo, d, h, mi, s, ms)))
}

/// `new Date()` — now | ms | ISO string | `(y, mo, d?, …)` UTC.
pub(crate) fn date_ctor(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let ms = match args.len() {
        0 => now_millis(),
        1 => match arg(args, 0) {
            Value::Str(id) => {
                let s = e.heap.string(id).unwrap_or_default();
                parse_js_date(&s).unwrap_or(f64::NAN)
            }
            v => e.to_number(v)?,
        },
        _ => {
            let mut n = |i: usize, dflt: i64| -> JsResult<i64> {
                Ok(match args.get(i) {
                    Some(v) => e.to_number(*v)? as i64,
                    None => dflt,
                })
            };
            parts_to_ms(
                n(0, 0)?,
                n(1, 0)?,
                n(2, 1)?,
                n(3, 0)?,
                n(4, 0)?,
                n(5, 0)?,
                n(6, 0)?,
            )
        }
    };
    let id = e.new_object();
    if let Some(o) = e.heap.obj_mut(id) {
        o.proto = Value::Obj(e.protos.date);
        o.set_own("__time", Value::Num(ms), false);
    }
    Ok(Value::Obj(id))
}

pub(crate) fn date_ms(e: &mut Engine, this: Value) -> JsResult<f64> {
    match this {
        Value::Obj(id) => match e.get_prop(id, "__time") {
            Some(Value::Num(n)) => Ok(n),
            _ => Err(e.type_error("Date method on non-Date")),
        },
        _ => Err(e.type_error("Date method on non-Date")),
    }
}

/// `d.toISOString()` → `YYYY-MM-DDTHH:mm:ss.sssZ` (NaN → `RangeError`).
pub(crate) fn date_to_iso(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    let ms = date_ms(e, this)?;
    if !ms.is_finite() {
        return Err(e.range_error("Invalid time value"));
    }
    let (y, mo, d, _wd, h, mi, s, mss) = ms_to_parts(ms);
    Ok(e.str_val(format!(
        "{y:04}-{mo:02}-{d:02}T{h:02}:{mi:02}:{s:02}.{mss:03}Z"
    )))
}

/// `d.toUTCString()` ≙ RFC-1123-ish `"Thu, 01 Jan 1970 00:00:00 GMT"`.
pub(crate) const WD_NAMES: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
pub(crate) const MO_NAMES: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

pub(crate) fn date_to_utc_string(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    let ms = date_ms(e, this)?;
    if !ms.is_finite() {
        return Ok(e.str_val("Invalid Date"));
    }
    let (y, mo, d, wd, h, mi, s, _m) = ms_to_parts(ms);
    Ok(e.str_val(format!(
        "{}, {:02} {} {:04} {:02}:{:02}:{:02} GMT",
        WD_NAMES[wd as usize],
        d,
        MO_NAMES[(mo - 1) as usize],
        y,
        h,
        mi,
        s
    )))
}

/// Shared `getX` — field index into `ms_to_parts` (0=y,1=mo-1,2=d,
/// 3=wd,4=h,5=mi,6=s,7=ms — month is 0-based JS-style).
pub(crate) fn date_part(e: &mut Engine, this: Value, i: usize) -> JsResult<f64> {
    let ms = date_ms(e, this)?;
    if !ms.is_finite() {
        return Ok(f64::NAN);
    }
    let p = ms_to_parts(ms);
    Ok(match i {
        0 => p.0 as f64,
        1 => f64::from(p.1) - 1.0,
        2 => f64::from(p.2),
        3 => f64::from(p.3),
        4 => f64::from(p.4),
        5 => f64::from(p.5),
        6 => f64::from(p.6),
        _ => f64::from(p.7),
    })
}

macro_rules! date_get {
    ($name:ident, $i:expr) => {
        pub(crate) fn $name(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
            Ok(Value::Num(date_part(e, this, $i)?))
        }
    };
}

date_get!(date_get_full_year, 0);
date_get!(date_get_month, 1);
date_get!(date_get_date, 2);
date_get!(date_get_day, 3);
date_get!(date_get_hours, 4);
date_get!(date_get_minutes, 5);
date_get!(date_get_seconds, 6);
date_get!(date_get_milliseconds, 7);

pub(crate) fn date_get_timezone_offset(
    _e: &mut Engine,
    _t: Value,
    _a: &[Value],
) -> JsResult<Value> {
    Ok(Value::Num(0.0)) // always UTC — no tz db in the engine
}

// -- JS-C19 — setters, formats chaînes, parse legacy -------------------------

/// Épaule `setX` — `first` = index de la 1re composante remplacée
/// (0=y, 1=mo 0-based, 2=d, 3=h, 4=mi, 5=s, 6=ms), `count` = nombre
/// d'args lus. Spec : args non-finis → date invalide ; date invalide
/// + setter → reste invalide. `setTime` est le seul à écrire brut.
pub(crate) fn date_set_parts(
    e: &mut Engine,
    this: Value,
    args: &[Value],
    first: usize,
    count: usize,
) -> JsResult<Value> {
    let Value::Obj(id) = this else {
        return Err(e.type_error("Date method on non-Date"));
    };
    let Some(Value::Num(ms)) = e.get_prop(id, "__time") else {
        return Err(e.type_error("Date method on non-Date"));
    };
    let mut vals = [0i64; 7];
    // Spec : le 1er arg est requis (absent -> ToNumber(undefined) ->
    // NaN -> date invalide) ; les suivants ne remplacent que s'ils
    // sont fournis - les composantes omises conservent leur valeur.
    let n_args = args.len().min(count).max(1);
    for (i, v) in vals.iter_mut().enumerate().take(n_args) {
        let n = e.to_number(arg(args, i))?;
        if !n.is_finite() {
            e.set_own(id, "__time", Value::Num(f64::NAN), false);
            return Ok(Value::Num(f64::NAN));
        }
        *v = n as i64;
    }
    if !ms.is_finite() {
        return Ok(Value::Num(f64::NAN));
    }
    let (y, mo, d, _wd, h, mi, sec, mss) = ms_to_parts(ms);
    let mut p = [
        y,
        i64::from(mo) - 1,
        i64::from(d),
        i64::from(h),
        i64::from(mi),
        i64::from(sec),
        i64::from(mss),
    ];
    for (i, v) in vals.iter().enumerate().take(n_args) {
        p[first + i] = *v;
    }
    // Chemin brut — pas le quirk `y<100 → 1900+y` de `parts_to_ms`
    // (spec : `setFullYear(50)` pose l'an 50 ; seuls le ctor,
    // `Date.UTC` et `setYear` mappent sur 1900).
    let new_ms = (days_from_civil(p[0], p[1] + 1, p[2]) * 86_400_000
        + p[3] * 3_600_000
        + p[4] * 60_000
        + p[5] * 1000
        + p[6]) as f64;
    e.set_own(id, "__time", Value::Num(new_ms), false);
    Ok(Value::Num(new_ms))
}

macro_rules! date_set {
    ($name:ident, $first:expr, $count:expr) => {
        pub(crate) fn $name(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
            date_set_parts(e, this, args, $first, $count)
        }
    };
}

date_set!(date_set_full_year, 0, 3);
date_set!(date_set_month, 1, 2);
date_set!(date_set_date, 2, 1);
date_set!(date_set_hours, 3, 4);
date_set!(date_set_minutes, 4, 3);
date_set!(date_set_seconds, 5, 2);
date_set!(date_set_ms, 6, 1);

/// `d.setTime(ms)` — écriture brute (NaN → invalide).
pub(crate) fn date_set_time(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let Value::Obj(id) = this else {
        return Err(e.type_error("Date method on non-Date"));
    };
    if e.get_prop(id, "__time").is_none() {
        return Err(e.type_error("Date method on non-Date"));
    }
    let n = e.to_number(arg(args, 0))?;
    e.set_own(id, "__time", Value::Num(n), false);
    Ok(Value::Num(n))
}

/// `d.toDateString()` → "Thu Jan 01 1970".
pub(crate) fn date_to_date_string(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    let ms = date_ms(e, this)?;
    Ok(e.str_val(if ms.is_finite() {
        let (y, mo, d, wd, _h, _mi, _s, _m) = ms_to_parts(ms);
        format!(
            "{} {} {:02} {:04}",
            WD_NAMES[wd as usize],
            MO_NAMES[(mo - 1) as usize],
            d,
            y
        )
    } else {
        "Invalid Date".into()
    }))
}

/// `d.toTimeString()` → "00:00:00 GMT+0000 (Coordinated Universal Time)".
pub(crate) fn date_to_time_string(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    let ms = date_ms(e, this)?;
    Ok(e.str_val(if ms.is_finite() {
        let (_y, _mo, _d, _wd, h, mi, sec, _m) = ms_to_parts(ms);
        format!("{h:02}:{mi:02}:{sec:02} GMT+0000 (Coordinated Universal Time)")
    } else {
        "Invalid Date".into()
    }))
}

/// `d.toJSON()` — spec `ToPrimitive` + appel `this.toISOString()`
/// (dispatchée : un `toISOString` réécrit par le script prime).
pub(crate) fn date_to_json(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    let ms = date_ms(e, this)?;
    if !ms.is_finite() {
        return Ok(Value::Null);
    }
    let f = e.get_prop_value(this, "toISOString")?;
    if !e.is_callable(f) {
        return Err(e.type_error("toJSON: toISOString is not callable"));
    }
    e.call_value(f, this, &[])
}

/// `d.toLocaleString()` — approximation `en-US` fixe
/// `"M/D/YYYY, h:mm:ss AM"` (pas de db locale dans le moteur).
pub(crate) fn date_to_locale_string(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    let ms = date_ms(e, this)?;
    Ok(e.str_val(if ms.is_finite() {
        let (y, mo, d, _wd, h, mi, sec, _m) = ms_to_parts(ms);
        let (h12, ampm) = ampm(h);
        format!("{mo}/{d}/{y}, {h12}:{mi:02}:{sec:02} {ampm}")
    } else {
        "Invalid Date".into()
    }))
}

/// `d.toLocaleDateString()` → `"M/D/YYYY"`.
pub(crate) fn date_to_locale_date(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    let ms = date_ms(e, this)?;
    Ok(e.str_val(if ms.is_finite() {
        let (y, mo, d, _wd, _h, _mi, _s, _m) = ms_to_parts(ms);
        format!("{mo}/{d}/{y}")
    } else {
        "Invalid Date".into()
    }))
}

/// `d.toLocaleTimeString()` → `"h:mm:ss AM"`.
pub(crate) fn date_to_locale_time(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    let ms = date_ms(e, this)?;
    Ok(e.str_val(if ms.is_finite() {
        let (_y, _mo, _d, _wd, h, mi, sec, _m) = ms_to_parts(ms);
        let (h12, ampm) = ampm(h);
        format!("{h12}:{mi:02}:{sec:02} {ampm}")
    } else {
        "Invalid Date".into()
    }))
}

/// `0 → 12 AM`, `13 → 1 PM` — format horaire `en-US`.
pub(crate) fn ampm(h: u32) -> (u32, &'static str) {
    let h12 = match h % 12 {
        0 => 12,
        n => n,
    };
    (h12, if h < 12 { "AM" } else { "PM" })
}

/// `Date.parse` complet : ISO strict d'abord, formats legacy ensuite.
pub(crate) fn parse_js_date(s: &str) -> Option<f64> {
    parse_iso_date(s).or_else(|| parse_legacy_date(s))
}

/// Parseur de dates legacy (Annex B interop) — formes couvertes :
/// `"Jan 15 2000"`, `"January 15, 2000"`, `"15 Jan 2000"`,
/// `"Thu Jan 01 1970 00:00:00 GMT+…"` (round-trip `toString`),
/// `"01/15/2000"` (M/D/Y US), heure `"h:m[:s[.ms]]"`, suffixes
/// `GMT`/`UTC` ignorés (fuseau unique). Année 2 chiffres :
/// `<50 → 20yy`, `≥50 → 19yy` (règle V8).
pub(crate) fn parse_legacy_date(s: &str) -> Option<f64> {
    const MO_FULL: [&str; 12] = [
        "january",
        "february",
        "march",
        "april",
        "may",
        "june",
        "july",
        "august",
        "september",
        "october",
        "november",
        "december",
    ];
    let month_of = |w: &str| -> Option<i64> {
        let w = w.to_ascii_lowercase();
        if w.len() < 3 || !w.bytes().all(|b| b.is_ascii_alphabetic()) {
            return None;
        }
        MO_FULL
            .iter()
            .position(|m| m.starts_with(&w))
            .map(|i| i as i64)
    };
    let mut y: Option<i64> = None;
    let mut mo: Option<i64> = None;
    let mut d: Option<i64> = None;
    let (mut h, mut mi, mut sec, mut mss) = (0i64, 0i64, 0i64, 0i64);
    let norm = s.replace(',', " ");
    for w in norm.split_whitespace() {
        if w.is_empty() {
            continue;
        }
        let lw = w.to_ascii_lowercase();
        if matches!(lw.as_str(), "gmt" | "utc")
            || lw.starts_with("gmt+")
            || lw.starts_with("gmt-")
            || WD_NAMES.iter().any(|n| n.eq_ignore_ascii_case(&lw))
        {
            continue;
        }
        if let Some(m) = month_of(&lw) {
            mo = Some(m + 1);
            continue;
        }
        if w.contains(':') {
            let mut tp = w.split(':');
            h = tp.next().and_then(|v| v.parse().ok())?;
            mi = tp.next().and_then(|v| v.parse().ok()).unwrap_or(0);
            if let Some(ss) = tp.next() {
                let (si, mf) = ss.split_once('.').map_or((ss, "0"), |(a, b)| (a, b));
                sec = si.parse().unwrap_or(0);
                mss = format!("{mf:0<3}")[..3].parse().unwrap_or(0);
            }
            continue;
        }
        if w.contains('/') {
            let parts: Vec<&str> = w.split('/').collect();
            let nums: Vec<i64> = parts.iter().filter_map(|v| v.parse().ok()).collect();
            if nums.len() != parts.len() || nums.len() < 2 || nums.len() > 3 {
                continue;
            }
            if parts[0].len() == 4 {
                // "YYYY/M/D" — année d'abord.
                y = Some(nums[0]);
                mo = Some(nums[1]);
                d = Some(if nums.len() == 3 { nums[2] } else { 1 });
            } else {
                // Ordre US M/D/Y — un mois >12 invalide (V8 pareil).
                mo = Some(nums[0]);
                d = Some(nums[1]);
                if nums.len() == 3 {
                    y = Some(nums[2]);
                }
            }
            continue;
        }
        // Nombre nu (ordinaires "1st"/"2nd" admis comme V8) ;
        // token non classifiable -> ignoré (tolérance legacy V8,
        // ex. le suffixe "(Coordinated Universal Time)" de
        // ).
        let digits = lw.trim_end_matches(|c: char| c.is_ascii_alphabetic());
        let Ok(n) = digits.parse::<i64>() else {
            continue;
        };
        if w.len() >= 4 || n > 31 {
            if y.is_none() {
                y = Some(n);
            }
        } else if d.is_none() {
            d = Some(n);
        } else if y.is_none() {
            y = Some(n);
        }
    }
    // Année 2 chiffres : <50 → 2000+n, ≥50 → 1900+n (règle V8).
    let y = match y? {
        n @ 0..=49 => 2000 + n,
        n @ 50..=99 => 1900 + n,
        n => n,
    };
    Some(
        (days_from_civil(y, mo?, d.unwrap_or(1)) * 86_400_000
            + h * 3_600_000
            + mi * 60_000
            + sec * 1000
            + mss) as f64,
    )
}
