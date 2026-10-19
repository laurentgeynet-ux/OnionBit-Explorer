// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! `Temporal` (ES2026) — tranche initiale bornée mais réelle.
//!
//! Implémenté : `Instant`, `PlainDate`, `PlainTime`,
//! `PlainDateTime`, `PlainYearMonth`, `PlainMonthDay`, `Duration`,
//! `Temporal.Now` (horloge UTC), `from`/`compare`/`with`/`equals`/
//! `add`/`subtract`/`since`/`until`, formats ISO-8601.
//!
//! Déviations documentées :
//! - calendrier unique `iso8601` (`calendarId` toujours
//!   `"iso8601"`, `era`/`eraYear` → `undefined`) ;
//! - fuseau unique UTC : `ZonedDateTime` et
//!   `Now.zonedDateTimeISO` absents, les offsets ISO sont
//!   appliqués arithmétiquement ;
//! - les props publiques (`year`, `hour`…) sont des props de
//!   données non-énumérables posées à la construction au lieu
//!   d'accesseurs de prototype — indistinguable hors
//!   `getOwnPropertyDescriptor` ;
//! - `PlainDate.add` ignore les champs sub-jour de la durée
//!   (la spec les balancerait via `relativeTo`) ;
//! - `Duration.compare`/`total` exigent l'absence de champs
//!   date non nuls sans `relativeTo` `PlainDate`/`PlainDateTime`
//!   (comme la spec).

use crate::builtins::{arg, c_id, ctor};
use crate::heap::{JsResult, NativeFn, TempData};
use crate::interp::Engine;
use crate::value::{ObjId, Value};
use num_traits::ToPrimitive;
use std::time::{SystemTime, UNIX_EPOCH};

// -- calendrier ISO ----------------------------------------------------

fn dim(y: i64, m: u8) -> u8 {
    match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap(y) => 29,
        2 => 28,
        _ => 0,
    }
}

fn leap(y: i64) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}

fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (m + 9).rem_euclid(12);
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn civil_from_days(z: i64) -> (i64, u8, u8) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u8;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u8;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// Fenêtre de validité spec (±(année ~275 760) autour de l'époque).
fn valid_date(y: i64, m: u8, d: u8) -> bool {
    if !(1..=12).contains(&m) || d < 1 || d > dim(y, m) {
        return false;
    }
    (-86_400_000_000_000_000i64..=86_400_000_000_000_000)
        .contains(&(days_from_civil(y, i64::from(m), i64::from(d)) * 86_400_000))
}

fn day_of_week(y: i64, m: u8, d: u8) -> u8 {
    // ISO : lundi = 1 … dimanche = 7 (epoch = jeudi).
    ((days_from_civil(y, i64::from(m), i64::from(d)).rem_euclid(7) + 3) % 7 + 1) as u8
}

fn day_of_year(y: i64, m: u8, d: u8) -> u16 {
    (days_from_civil(y, i64::from(m), i64::from(d)) - days_from_civil(y, 1, 1) + 1) as u16
}

/// Semaine ISO-8601 (`weekOfYear`) + `yearOfWeek`.
fn iso_week(y: i64, m: u8, d: u8) -> (u8, i64) {
    let doy = i64::from(day_of_year(y, m, d));
    let dow = i64::from(day_of_week(y, m, d));
    let week = (doy - dow + 10).div_euclid(7);
    if week < 1 {
        return iso_week(y - 1, 12, 31);
    }
    let weeks_in_year = {
        // 53 si l'année commence un jeudi (ou mercredi bissextile).
        let d1 = day_of_week(y, 1, 1);
        u8::from(d1 == 4 || (d1 == 3 && leap(y))) + 52
    };
    if week > i64::from(weeks_in_year) {
        (1, y + 1)
    } else {
        (week as u8, y)
    }
}

// -- lecture des slots internes ----------------------------------------

fn td(e: &mut Engine, this: Value) -> JsResult<TempData> {
    if let Value::Obj(id) = this {
        if let Some(crate::heap::ObjKind::Temp(t)) = e.heap.obj(id).map(|o| &o.kind) {
            return Ok(*t);
        }
    }
    Err(e.type_error("Temporal method on incompatible receiver"))
}

/// `(ObjId, TempData)` — pour les méthodes qui modifient.
fn td_id(e: &mut Engine, this: Value) -> JsResult<(ObjId, TempData)> {
    if let Value::Obj(id) = this {
        if let Some(crate::heap::ObjKind::Temp(t)) = e.heap.obj(id).map(|o| &o.kind) {
            return Ok((id, *t));
        }
    }
    Err(e.type_error("Temporal method on incompatible receiver"))
}

// -- construction / props ----------------------------------------------

/// Proto `Temporal.<Name>.prototype` via la chaîne globale.
fn tproto(e: &mut Engine, name: &str) -> JsResult<ObjId> {
    let Some(Value::Obj(t)) = e.get_prop(e.global, "Temporal") else {
        return Err(e.type_error("Temporal missing"));
    };
    let c = e
        .get_prop(t, name)
        .and_then(|v| match v {
            Value::Obj(id) => Some(id),
            _ => None,
        })
        .ok_or_else(|| e.type_error("Temporal constructor missing"))?;
    match e.get_prop(c, "prototype") {
        Some(Value::Obj(p)) => Ok(p),
        _ => Err(e.type_error("Temporal prototype missing")),
    }
}

fn mk(e: &mut Engine, name: &str, data: TempData) -> JsResult<Value> {
    let proto = Value::Obj(tproto(e, name)?);
    let id = e.heap.alloc_obj(crate::heap::Object {
        proto,
        props: Vec::new(),
        kind: crate::heap::ObjKind::Temp(data),
        extensible: true,
        elem_integrity: 0,
    });
    set_pub_props(e, id, &data);
    Ok(Value::Obj(id))
}

/// Props publiques (data, non-énumérables — déviation vs
/// accesseurs proto, documentée en tête de module).
fn set_pub_props(e: &mut Engine, id: ObjId, t: &TempData) {
    let num = |n: f64| Value::Num(n);
    match t {
        TempData::Instant { ns } => {
            p(
                e,
                id,
                "epochMilliseconds",
                num(ns.div_euclid(1_000_000) as f64),
            );
            let b = e.big_val(num_bigint::BigInt::from(*ns));
            p(e, id, "epochNanoseconds", b);
        }
        TempData::PDate { y, m, d } => date_props(e, id, *y, *m, *d),
        TempData::PTime {
            h,
            mi,
            s_,
            ms,
            us,
            ns,
        } => {
            time_props(e, id, *h, *mi, *s_, *ms, *us, *ns);
        }
        TempData::PDateTime {
            y,
            m,
            d,
            h,
            mi,
            s_,
            ms,
            us,
            ns,
        } => {
            date_props(e, id, *y, *m, *d);
            time_props(e, id, *h, *mi, *s_, *ms, *us, *ns);
        }
        TempData::PYearMonth { y, m } => {
            p(e, id, "year", num(*y as f64));
            p(e, id, "month", num(f64::from(*m)));
            let mc = e.str_val(format!("M{m:02}"));
            p(e, id, "monthCode", mc);
            p(e, id, "daysInMonth", num(f64::from(dim(*y, *m))));
            p(e, id, "monthsInYear", num(12.0));
            p(e, id, "inLeapYear", Value::Bool(leap(*y)));
            let cal = e.str_val("iso8601");
            p(e, id, "calendarId", cal);
        }
        TempData::PMonthDay { m, d } => {
            let mc = e.str_val(format!("M{m:02}"));
            p(e, id, "monthCode", mc);
            p(e, id, "day", num(f64::from(*d)));
            let cal = e.str_val("iso8601");
            p(e, id, "calendarId", cal);
        }
        TempData::Duration { f } => {
            for (k, v) in [
                ("years", f[0]),
                ("months", f[1]),
                ("weeks", f[2]),
                ("days", f[3]),
                ("hours", f[4]),
                ("minutes", f[5]),
                ("seconds", f[6]),
                ("milliseconds", f[7]),
                ("microseconds", f[8]),
                ("nanoseconds", f[9]),
            ] {
                p(e, id, k, num(v as f64));
            }
            p(e, id, "sign", num(f64::from(dur_sign(f))));
        }
    }
}

fn p(e: &mut Engine, id: ObjId, k: &str, v: Value) {
    e.set_own(id, k, v, false);
}

fn date_props(e: &mut Engine, id: ObjId, y: i64, m: u8, d: u8) {
    let num = |n: f64| Value::Num(n);
    p(e, id, "year", num(y as f64));
    p(e, id, "month", num(f64::from(m)));
    let mc = e.str_val(format!("M{m:02}"));
    p(e, id, "monthCode", mc);
    p(e, id, "day", num(f64::from(d)));
    p(e, id, "dayOfWeek", num(f64::from(day_of_week(y, m, d))));
    p(e, id, "dayOfYear", num(f64::from(day_of_year(y, m, d))));
    let (w, wy) = iso_week(y, m, d);
    p(e, id, "weekOfYear", num(f64::from(w)));
    p(e, id, "yearOfWeek", num(wy as f64));
    p(e, id, "daysInWeek", num(7.0));
    p(e, id, "daysInMonth", num(f64::from(dim(y, m))));
    p(
        e,
        id,
        "daysInYear",
        num(if leap(y) { 366.0 } else { 365.0 }),
    );
    p(e, id, "monthsInYear", num(12.0));
    p(e, id, "inLeapYear", Value::Bool(leap(y)));
    let cal = e.str_val("iso8601");
    p(e, id, "calendarId", cal);
}

#[allow(clippy::too_many_arguments)] // 6 champs temps —
                                     // tuple artificiel moins lisible que la signature plate.
fn time_props(e: &mut Engine, id: ObjId, h: u8, mi: u8, s_: u8, ms: u16, us: u16, ns: u16) {
    let num = |n: f64| Value::Num(n);
    p(e, id, "hour", num(f64::from(h)));
    p(e, id, "minute", num(f64::from(mi)));
    p(e, id, "second", num(f64::from(s_)));
    p(e, id, "millisecond", num(f64::from(ms)));
    p(e, id, "microsecond", num(f64::from(us)));
    p(e, id, "nanosecond", num(f64::from(ns)));
}

fn dur_sign(f: &[i64; 10]) -> i32 {
    if f.iter().any(|&x| x > 0) {
        1
    } else if f.iter().any(|&x| x < 0) {
        -1
    } else {
        0
    }
}

// -- conversion d'arguments --------------------------------------------

fn int_arg(e: &mut Engine, v: Value, what: &str) -> JsResult<i64> {
    let n = e.to_number(v)?;
    if !n.is_finite() || n.fract() != 0.0 {
        return Err(e.range_error(format!("{what} must be a finite integer")));
    }
    Ok(n as i64)
}

fn int_arg_def(e: &mut Engine, args: &[Value], i: usize, def: i64, what: &str) -> JsResult<i64> {
    match arg(args, i) {
        Value::Undefined => Ok(def),
        v => int_arg(e, v, what),
    }
}

/// Lit un champ entier d'un objet-props (`from`/`with`).
fn field(e: &mut Engine, o: ObjId, key: &str) -> JsResult<Option<i64>> {
    match e.get_prop(o, key) {
        Some(Value::Undefined) | None => Ok(None),
        Some(v) => Ok(Some(int_arg(e, v, key)?)),
    }
}

/// `options.largestUnit` (défaut `def`).
fn largest_unit(e: &mut Engine, opts: Value, def: &'static str) -> JsResult<String> {
    if let Value::Obj(id) = opts {
        if let Some(v) = e.get_prop(id, "largestUnit") {
            if !matches!(v, Value::Undefined) {
                return e.to_js_string(v);
            }
        }
    }
    Ok(def.to_string())
}

// -- formats ISO --------------------------------------------------------

fn fmt_y(y: i64) -> String {
    if (0..=9999).contains(&y) {
        format!("{y:04}")
    } else {
        format!("{}{:06}", if y < 0 { "-" } else { "+" }, y.abs())
    }
}

fn fmt_frac(ms: u16, us: u16, ns: u16) -> String {
    if ns != 0 {
        format!(
            ".{:09}",
            u64::from(ms) * 1_000_000 + u64::from(us) * 1_000 + u64::from(ns)
        )
    } else if us != 0 {
        format!(".{:06}", u32::from(ms) * 1_000 + u32::from(us))
    } else if ms != 0 {
        format!(".{ms:03}")
    } else {
        String::new()
    }
}

fn fmt_time(h: u8, mi: u8, s: u8, ms: u16, us: u16, ns: u16) -> String {
    format!("{h:02}:{mi:02}:{s:02}{}", fmt_frac(ms, us, ns))
}

fn to_string_of(_e: &Engine, t: &TempData) -> String {
    match t {
        TempData::Instant { ns } => {
            let ms = ns.div_euclid(1_000_000);
            let (y, m, d, _w, h, mi, s, mss) = crate::builtins::ms_to_parts(ms as f64);
            let rem = ns.rem_euclid(1_000_000) as u32;
            let (fms, fus, fns) = (
                mss as u16,
                (rem / 1_000) as u16 % 1_000,
                (rem % 1_000) as u16,
            );
            format!(
                "{}-{:02}-{:02}T{:02}:{:02}:{:02}{}Z",
                fmt_y(y),
                m,
                d,
                h,
                mi,
                s,
                fmt_frac(fms, fus, fns)
            )
        }
        TempData::PDate { y, m, d } => format!("{}-{:02}-{:02}", fmt_y(*y), m, d),
        TempData::PTime {
            h,
            mi,
            s_,
            ms,
            us,
            ns,
        } => fmt_time(*h, *mi, *s_, *ms, *us, *ns),
        TempData::PDateTime {
            y,
            m,
            d,
            h,
            mi,
            s_,
            ms,
            us,
            ns,
        } => format!(
            "{}-{:02}-{:02}T{}",
            fmt_y(*y),
            m,
            d,
            fmt_time(*h, *mi, *s_, *ms, *us, *ns)
        ),
        TempData::PYearMonth { y, m } => format!("{}-{:02}", fmt_y(*y), m),
        TempData::PMonthDay { m, d } => format!("{m:02}-{d:02}"),
        TempData::Duration { f } => {
            if dur_sign(f) == 0 {
                return "PT0S".to_string();
            }
            let sgn = if f.iter().all(|&x| x <= 0) { "-" } else { "" };
            let a = |x: i64| x.abs();
            let mut out = format!("{sgn}P");
            if f[0] != 0 {
                out += &format!("{}Y", a(f[0]));
            }
            if f[1] != 0 {
                out += &format!("{}M", a(f[1]));
            }
            if f[2] != 0 {
                out += &format!("{}W", a(f[2]));
            }
            if f[3] != 0 {
                out += &format!("{}D", a(f[3]));
            }
            let has_t = f[4..].iter().any(|&x| x != 0);
            if has_t {
                out.push('T');
            }
            if f[4] != 0 {
                out += &format!("{}H", a(f[4]));
            }
            if f[5] != 0 {
                out += &format!("{}M", a(f[5]));
            }
            let (s, ms, us, ns) = (f[6], f[7], f[8], f[9]);
            if s != 0 || ms != 0 || us != 0 || ns != 0 || !has_t && f[..4].iter().all(|&x| x == 0) {
                let frac = fmt_frac(
                    ms.unsigned_abs() as u16,
                    us.unsigned_abs() as u16,
                    ns.unsigned_abs() as u16,
                );
                out += &format!("{}{}S", a(s), frac);
            }
            out
        }
    }
}

// -- parsing ISO ---------------------------------------------------------

/// `YYYY-MM-DD` ou `YYYYMMDD` (année signée 4/6 chiffres).
fn parse_date_part(s: &str) -> Option<(i64, u8, u8)> {
    let (y, rest) = if let Some(r) = s.strip_prefix(['+', '-']) {
        let (a, b) = r.split_at(6.min(r.len()));
        (s[..1].to_string() + a, &b[6.min(b.len())..])
    } else if s.len() >= 5 && s.as_bytes()[4] == b'-' {
        (s[..4].to_string(), &s[4..])
    } else {
        (s[..4.min(s.len())].to_string(), &s[4.min(s.len())..])
    };
    // Forme non-dashed : YYYYMMDD.
    let y: i64 = y.parse().ok()?;
    let rest = rest.strip_prefix('-').unwrap_or(rest);
    if rest.len() < 2 {
        return None;
    }
    let m: u8 = rest[..2].parse().ok()?;
    let rest = &rest[2..];
    let rest = rest.strip_prefix('-').unwrap_or(rest);
    let d: u8 = if rest.is_empty() {
        1
    } else if rest.len() >= 2 {
        rest[..2].parse().ok()?
    } else {
        rest.parse().ok()?
    };
    Some((y, m, d))
}

/// `HH:MM[:SS[.ffff]]` ou `HHMM[SS[.ffff]]` — retourne
/// `(h, mi, s, ms, us, ns, reste)` (reste = offset/`Z` éventuel).
fn parse_time_part(s: &str) -> Option<(u8, u8, u8, u16, u16, u16, &str)> {
    let b = s.as_bytes();
    if b.len() < 4 {
        return None;
    }
    let h: u8 = s.get(0..2)?.parse().ok()?;
    let (mi, mut i) = if b.get(2) == Some(&b':') {
        (s.get(3..5)?.parse().ok()?, 5)
    } else {
        (s.get(2..4)?.parse().ok()?, 4)
    };
    let mut s_ = 0u8;
    let (mut ms, mut us, mut ns) = (0u16, 0u16, 0u16);
    if b.get(i) == Some(&b':') {
        s_ = s.get(i + 1..i + 3)?.parse().ok()?;
        i += 3;
    } else if i == 4 && b.len() >= 6 && b[i].is_ascii_digit() {
        s_ = s.get(i..i + 2)?.parse().ok()?;
        i += 2;
    }
    if b.get(i) == Some(&b'.') || b.get(i) == Some(&b',') {
        i += 1;
        let st = i;
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
        }
        let frac = &s[st..i];
        let f = format!("{frac:0<9}");
        let n: u32 = f.parse().ok()?;
        ms = (n / 1_000_000) as u16;
        us = (n / 1_000 % 1_000) as u16;
        ns = (n % 1_000) as u16;
    }
    Some((h, mi, s_, ms, us, ns, &s[i..]))
}

/// `Z` ou `±HH[:MM]` → minutes d'offset (appliquées à l'instant).
fn parse_offset(s: &str) -> Option<i64> {
    let s = s.trim_start_matches(['[', ']']);
    if s.is_empty() || s.eq_ignore_ascii_case("z") {
        return Some(0);
    }
    let (sgn, r) = match s.as_bytes().first()? {
        b'+' => (1i64, &s[1..]),
        b'-' => (-1i64, &s[1..]),
        _ => return None,
    };
    let r = r.split('[').next().unwrap_or(r);
    let h: i64 = r.get(0..2)?.parse().ok()?;
    // Minutes optionnelles : `+02`, `+0230`, `+02:30`.
    let rest = &r[2..];
    let rest = rest.strip_prefix(':').unwrap_or(rest);
    let mi = if rest.is_empty() {
        0
    } else {
        rest.get(0..2)?.parse().ok()?
    };
    Some(sgn * (h * 60 + mi))
}

/// `PnYnMnWnDTnHnMn.fS` — fraction permise sur les secondes
/// (et sur l'unité minimale présente — approximation documentée).
fn parse_duration(s: &str) -> Option<[i64; 10]> {
    let (sgn, s) = if let Some(r) = s.strip_prefix('-') {
        (-1i64, r)
    } else if let Some(r) = s.strip_prefix('+') {
        (1i64, r)
    } else {
        (1, s)
    };
    let s = s.strip_prefix('P').or_else(|| s.strip_prefix('p'))?;
    let (date, time) = s.split_once(['T', 't']).map_or((s, ""), |(a, b)| (a, b));
    let mut f = [0i64; 10];
    let take_num = |x: &mut &str| -> Option<String> {
        let mut n = String::new();
        while let Some(c) = x.chars().next() {
            if c.is_ascii_digit() || (c == '.' && !n.contains('.')) {
                n.push(c);
                *x = &x[1..];
            } else {
                break;
            }
        }
        (!n.is_empty()).then_some(n)
    };
    let mut d = date;
    let mut seen = false;
    while !d.is_empty() {
        let n = take_num(&mut d)?;
        let u = d.chars().next()?;
        d = &d[1..];
        seen = true;
        match u {
            'Y' | 'y' => f[0] = n.parse().ok()?,
            'M' | 'm' => f[1] = n.parse().ok()?,
            'W' | 'w' => f[2] = n.parse().ok()?,
            'D' | 'd' => f[3] = n.parse().ok()?,
            _ => return None,
        }
    }
    let mut t = time;
    while !t.is_empty() {
        let n = take_num(&mut t)?;
        let u = t.chars().next()?;
        t = &t[1..];
        seen = true;
        match u {
            'H' | 'h' => f[4] = n.parse().ok()?,
            'M' | 'm' => {
                if let Some((i, fr)) = n.split_once('.') {
                    f[5] = i.parse().ok()?;
                    let fl: f64 = format!("0.{fr}").parse().ok()?;
                    f[6] = (fl * 60.0) as i64;
                } else {
                    f[5] = n.parse().ok()?;
                }
            }
            'S' | 's' => {
                if let Some((i, fr)) = n.split_once('.') {
                    f[6] = i.parse().ok()?;
                    let pad = format!("{fr:0<9}");
                    let nano: i64 = pad.parse().ok()?;
                    f[7] = nano / 1_000_000;
                    f[8] = nano / 1_000 % 1_000;
                    f[9] = nano % 1_000;
                } else {
                    f[6] = n.parse().ok()?;
                }
            }
            _ => return None,
        }
    }
    if !seen {
        return None;
    }
    for x in &mut f {
        *x *= sgn;
    }
    Some(f)
}

/// Durée → `TempData::Duration` : string ISO, objet-props, ou
/// instance Duration (spec `ToTemporalDuration`).
fn to_duration(e: &mut Engine, v: Value) -> JsResult<[i64; 10]> {
    match v {
        Value::Str(id) => {
            let s = e.heap.string(id).unwrap_or_default();
            parse_duration(&s).ok_or_else(|| e.range_error("invalid duration string"))
        }
        Value::Obj(id) => {
            if let Some(crate::heap::ObjKind::Temp(TempData::Duration { f })) =
                e.heap.obj(id).map(|o| &o.kind)
            {
                return Ok(*f);
            }
            // Spec : `v` peut aussi être un objet props
            // {years, months, …}.
            let mut f = [0i64; 10];
            let mut any = false;
            for (i, k) in [
                "years",
                "months",
                "weeks",
                "days",
                "hours",
                "minutes",
                "seconds",
                "milliseconds",
                "microseconds",
                "nanoseconds",
            ]
            .iter()
            .enumerate()
            {
                if let Some(x) = field(e, id, k)? {
                    f[i] = x;
                    any = true;
                }
            }
            if !any {
                return Err(e.type_error("invalid duration"));
            }
            if f.iter().any(|&x| x > 0) && f.iter().any(|&x| x < 0) {
                return Err(e.range_error("duration fields must have the same sign"));
            }
            Ok(f)
        }
        _ => Err(e.type_error("duration expected")),
    }
}

// -- arithmétique -------------------------------------------------------

/// `(y, m, d)` + `(dy, dmo, dd)` calendrier avec clamp du jour.
fn add_ymd(y: i64, m: u8, d: u8, dy: i64, dmo: i64, dd: i64) -> (i64, u8, u8) {
    let total_m = (y * 12 + i64::from(m) - 1) + dy * 12 + dmo;
    let ny = total_m.div_euclid(12);
    let nm = (total_m.rem_euclid(12) + 1) as u8;
    let nd = d.min(dim(ny, nm));
    let days = days_from_civil(ny, i64::from(nm), i64::from(nd)) + dd;
    civil_from_days(days)
}

/// Durée temps (champs 4..10) → nanosecondes i128.
fn dur_time_ns(f: &[i64; 10]) -> i128 {
    i128::from(f[4]) * 3_600_000_000_000
        + i128::from(f[5]) * 60_000_000_000
        + i128::from(f[6]) * 1_000_000_000
        + i128::from(f[7]) * 1_000_000
        + i128::from(f[8]) * 1_000
        + i128::from(f[9])
}

/// `(h,mi,s,ms,us,ns)` + `ns_delta` modulo 24h → `(carry_jours, time)`.
fn add_time(
    h: u8,
    mi: u8,
    s: u8,
    ms: u16,
    us: u16,
    ns: u16,
    delta: i128,
) -> (i64, (u8, u8, u8, u16, u16, u16)) {
    let base = i128::from(h) * 3_600_000_000_000
        + i128::from(mi) * 60_000_000_000
        + i128::from(s) * 1_000_000_000
        + i128::from(ms) * 1_000_000
        + i128::from(us) * 1_000
        + i128::from(ns);
    let day = 86_400_000_000_000i128;
    let total = base + delta;
    let carry = total.div_euclid(day) as i64;
    let rem = total.rem_euclid(day);
    (
        carry,
        (
            (rem / 3_600_000_000_000) as u8,
            (rem / 60_000_000_000 % 60) as u8,
            (rem / 1_000_000_000 % 60) as u8,
            (rem / 1_000_000 % 1_000) as u16,
            (rem / 1_000 % 1_000) as u16,
            (rem % 1_000) as u16,
        ),
    )
}

/// `ns` → `[0,0,0,0,h,mi,s,ms,us,ns]` selon `largest`
/// (hours/minutes/seconds/milliseconds/microseconds/nanoseconds).
fn ns_to_dur(mut ns: i128, largest: &str) -> [i64; 10] {
    let mut f = [0i64; 10];
    let units: [(&str, i128); 6] = [
        ("hours", 3_600_000_000_000),
        ("minutes", 60_000_000_000),
        ("seconds", 1_000_000_000),
        ("milliseconds", 1_000_000),
        ("microseconds", 1_000),
        ("nanoseconds", 1),
    ];
    let start = units.iter().position(|(u, _)| *u == largest).unwrap_or(2);
    for (i, (_, u)) in units.iter().enumerate().skip(start) {
        f[4 + i] = (ns / u) as i64;
        ns %= u;
    }
    f
}

/// Différence calendaire `(y1,m1,d1)→(y2,m2,d2)` en
/// `(years, months, days)` — algorithme spec `DifferenceDate`.
fn diff_date(
    y1: i64,
    m1: u8,
    d1: u8,
    y2: i64,
    m2: u8,
    d2: u8,
    years_months: bool,
) -> (i64, i64, i64) {
    if !years_months {
        return (
            0,
            0,
            days_from_civil(y2, i64::from(m2), i64::from(d2))
                - days_from_civil(y1, i64::from(m1), i64::from(d1)),
        );
    }
    let mut months = (y2 - y1) * 12 + i64::from(m2) - i64::from(m1);
    let cand = add_ymd(y1, m1, d1, 0, months, 0);
    let cd = days_from_civil(cand.0, i64::from(cand.1), i64::from(cand.2));
    let dd = days_from_civil(y2, i64::from(m2), i64::from(d2)) - cd;
    if months != 0 && months.signum() != dd.signum() && dd != 0 {
        months -= months.signum();
    }
    let cand = add_ymd(y1, m1, d1, 0, months, 0);
    let cd = days_from_civil(cand.0, i64::from(cand.1), i64::from(cand.2));
    let dd = days_from_civil(y2, i64::from(m2), i64::from(d2)) - cd;
    (months.div_euclid(12), months.rem_euclid(12), dd)
}

/// `options.relativeTo` → `(y,m,d)` ou `None`.
fn relative_to(e: &mut Engine, opts: Value) -> JsResult<Option<(i64, u8, u8)>> {
    if let Value::Obj(id) = opts {
        if let Some(v) = e.get_prop(id, "relativeTo") {
            if !matches!(v, Value::Undefined) {
                let t = to_pdate(e, v)?;
                if let TempData::PDate { y, m, d } = t {
                    return Ok(Some((y, m, d)));
                }
            }
        }
    }
    Ok(None)
}

// -- ToTemporalX (arg → slots) -------------------------------------------

fn to_pdate(e: &mut Engine, v: Value) -> JsResult<TempData> {
    match v {
        Value::Obj(id) => match e.heap.obj(id).map(|o| &o.kind) {
            Some(crate::heap::ObjKind::Temp(t)) => {
                let r = match *t {
                    x @ TempData::PDate { .. } => x,
                    TempData::PDateTime { y, m, d, .. } => TempData::PDate { y, m, d },
                    _ => {
                        return Err(e.type_error("PlainDate expected"));
                    }
                };
                Ok(r)
            }
            Some(_) => {
                // Objet-props {year, month|monthCode, day} — ou
                // objet avec calendarId (spec lirait le calendrier ;
                // iso8601 unique chez nous).
                let y = field(e, id, "year")?;
                let m = match field(e, id, "month")? {
                    Some(x) => Some(x),
                    None => match e.get_prop(id, "monthCode") {
                        Some(Value::Str(sid)) => {
                            let s = e.heap.string(sid).unwrap_or_default();
                            s.strip_prefix('M').and_then(|x| x.parse::<i64>().ok())
                        }
                        _ => None,
                    },
                };
                let d = field(e, id, "day")?;
                match (y, m, d) {
                    (Some(y), Some(m), Some(d)) => {
                        let (m, d) = (m as u8, d as u8);
                        if !valid_date(y, m, d) {
                            return Err(e.range_error("invalid date"));
                        }
                        Ok(TempData::PDate { y, m, d })
                    }
                    _ => Err(e.type_error("PlainDate fields missing")),
                }
            }
            None => Err(e.type_error("PlainDate expected")),
        },
        Value::Str(id) => {
            let s = e.heap.string(id).unwrap_or_default();
            let date = s.split(['T', ' ']).next().unwrap_or(&s);
            match parse_date_part(date) {
                Some((y, m, d)) if valid_date(y, m, d) => Ok(TempData::PDate { y, m, d }),
                _ => Err(e.range_error("invalid PlainDate string")),
            }
        }
        _ => Err(e.type_error("PlainDate expected")),
    }
}

fn to_ptime(e: &mut Engine, v: Value) -> JsResult<TempData> {
    match v {
        Value::Obj(id) => match e.heap.obj(id).map(|o| &o.kind) {
            Some(crate::heap::ObjKind::Temp(t)) => {
                let r = match *t {
                    x @ TempData::PTime { .. } => x,
                    TempData::PDateTime {
                        h,
                        mi,
                        s_,
                        ms,
                        us,
                        ns,
                        ..
                    } => TempData::PTime {
                        h,
                        mi,
                        s_,
                        ms,
                        us,
                        ns,
                    },
                    _ => return Err(e.type_error("PlainTime expected")),
                };
                Ok(r)
            }
            Some(_) => {
                let h = field(e, id, "hour")?.unwrap_or(0);
                let mi = field(e, id, "minute")?.unwrap_or(0);
                let s_ = field(e, id, "second")?.unwrap_or(0);
                let ms = field(e, id, "millisecond")?.unwrap_or(0);
                let us = field(e, id, "microsecond")?.unwrap_or(0);
                let ns = field(e, id, "nanosecond")?.unwrap_or(0);
                let (h, mi, s_, ms, us, ns) =
                    (h as u8, mi as u8, s_ as u8, ms as u16, us as u16, ns as u16);
                if h > 23 || mi > 59 || s_ > 59 || ms > 999 || us > 999 || ns > 999 {
                    return Err(e.range_error("invalid time"));
                }
                Ok(TempData::PTime {
                    h,
                    mi,
                    s_,
                    ms,
                    us,
                    ns,
                })
            }
            None => Err(e.type_error("PlainTime expected")),
        },
        Value::Str(id) => {
            let s = e.heap.string(id).unwrap_or_default();
            let s = s.strip_prefix('T').unwrap_or(&s);
            match parse_time_part(s) {
                Some((h, mi, s_, ms, us, ns, _)) if h <= 23 && mi <= 59 && s_ <= 59 => {
                    Ok(TempData::PTime {
                        h,
                        mi,
                        s_,
                        ms,
                        us,
                        ns,
                    })
                }
                _ => Err(e.range_error("invalid PlainTime string")),
            }
        }
        _ => Err(e.type_error("PlainTime expected")),
    }
}

fn to_pdatetime(e: &mut Engine, v: Value) -> JsResult<TempData> {
    match v {
        Value::Obj(id) => match e.heap.obj(id).map(|o| &o.kind) {
            Some(crate::heap::ObjKind::Temp(t)) => {
                let r = match *t {
                    x @ TempData::PDateTime { .. } => x,
                    TempData::PDate { y, m, d } => TempData::PDateTime {
                        y,
                        m,
                        d,
                        h: 0,
                        mi: 0,
                        s_: 0,
                        ms: 0,
                        us: 0,
                        ns: 0,
                    },
                    _ => return Err(e.type_error("PlainDateTime expected")),
                };
                Ok(r)
            }
            Some(_) => {
                let y = field(e, id, "year")?
                    .ok_or_else(|| e.type_error("PlainDateTime fields missing"))?;
                let m = field(e, id, "month")?.unwrap_or(1) as u8;
                let d = field(e, id, "day")?.unwrap_or(1) as u8;
                let h = field(e, id, "hour")?.unwrap_or(0) as u8;
                let mi = field(e, id, "minute")?.unwrap_or(0) as u8;
                let s_ = field(e, id, "second")?.unwrap_or(0) as u8;
                let ms = field(e, id, "millisecond")?.unwrap_or(0) as u16;
                let us = field(e, id, "microsecond")?.unwrap_or(0) as u16;
                let ns = field(e, id, "nanosecond")?.unwrap_or(0) as u16;
                if !valid_date(y, m, d)
                    || h > 23
                    || mi > 59
                    || s_ > 59
                    || ms > 999
                    || us > 999
                    || ns > 999
                {
                    return Err(e.range_error("invalid datetime"));
                }
                Ok(TempData::PDateTime {
                    y,
                    m,
                    d,
                    h,
                    mi,
                    s_,
                    ms,
                    us,
                    ns,
                })
            }
            None => Err(e.type_error("PlainDateTime expected")),
        },
        Value::Str(id) => {
            let s = e.heap.string(id).unwrap_or_default();
            let (dp, tp) = s
                .split_once(['T', ' '])
                .map_or((s.as_str(), ""), |(a, b)| (a, b));
            let (y, m, d) =
                parse_date_part(dp).ok_or_else(|| e.range_error("invalid PlainDateTime string"))?;
            let (h, mi, s_, ms, us, ns) = if tp.is_empty() {
                (0, 0, 0, 0, 0, 0)
            } else {
                let (h, mi, s_, ms, us, ns, _rest) = parse_time_part(tp)
                    .ok_or_else(|| e.range_error("invalid PlainDateTime string"))?;
                (h, mi, s_, ms, us, ns)
            };
            if !valid_date(y, m, d) || h > 23 || mi > 59 || s_ > 59 {
                return Err(e.range_error("invalid PlainDateTime string"));
            }
            Ok(TempData::PDateTime {
                y,
                m,
                d,
                h,
                mi,
                s_,
                ms,
                us,
                ns,
            })
        }
        _ => Err(e.type_error("PlainDateTime expected")),
    }
}

fn to_instant(e: &mut Engine, v: Value) -> JsResult<TempData> {
    match v {
        Value::Obj(id) => match e.heap.obj(id).map(|o| &o.kind) {
            Some(crate::heap::ObjKind::Temp(t @ TempData::Instant { .. })) => Ok(*t),
            _ => Err(e.type_error("Instant expected")),
        },
        Value::Str(id) => {
            let s = e.heap.string(id).unwrap_or_default();
            let (dp, tp) = s
                .split_once(['T', ' '])
                .map_or((s.as_str(), ""), |(a, b)| (a, b));
            let (y, m, d) =
                parse_date_part(dp).ok_or_else(|| e.range_error("invalid Instant string"))?;
            let (h, mi, s_, ms, us, ns, off) =
                parse_time_part(tp).ok_or_else(|| e.range_error("invalid Instant string"))?;
            // Spec : offset ou `Z` obligatoire — on accepte aussi
            // l'absence (= UTC, tolérance documentée).
            let off_min = parse_offset(off).unwrap_or(0);
            if !valid_date(y, m, d) || h > 23 || mi > 59 || s_ > 59 {
                return Err(e.range_error("invalid Instant string"));
            }
            let base_ms = days_from_civil(y, i64::from(m), i64::from(d)) * 86_400_000
                + i64::from(h) * 3_600_000
                + i64::from(mi) * 60_000
                + i64::from(s_) * 1_000
                + i64::from(ms)
                - off_min * 60_000;
            let ns = i128::from(base_ms) * 1_000_000 + i128::from(us) * 1_000 + i128::from(ns);
            Ok(TempData::Instant { ns })
        }
        _ => Err(e.type_error("Instant expected")),
    }
}

// -- méthodes génériques --------------------------------------------------

fn t_to_string(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    let t = td(e, this)?;
    let st = to_string_of(e, &t);
    Ok(e.str_val(st))
}

fn t_to_json(e: &mut Engine, this: Value, a: &[Value]) -> JsResult<Value> {
    t_to_string(e, this, a)
}

/// `valueOf` lève `TypeError` sur tout Temporal (spec — la
/// conversion implicite est interdite).
fn t_value_of(e: &mut Engine, _t: Value, _a: &[Value]) -> JsResult<Value> {
    Err(e.type_error("Temporal objects cannot be converted implicitly"))
}

fn t_equals(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let t = td(e, this)?;
    let o = to_same(e, &t, arg(args, 0))?;
    Ok(Value::Bool(temporal_cmp(&t, &o) == 0))
}

/// Convertit `v` vers le même `TempData` que `t`.
fn to_same(e: &mut Engine, t: &TempData, v: Value) -> JsResult<TempData> {
    match t {
        TempData::Instant { .. } => to_instant(e, v),
        TempData::PDate { .. } => to_pdate(e, v),
        TempData::PTime { .. } => to_ptime(e, v),
        TempData::PDateTime { .. } => to_pdatetime(e, v),
        TempData::PYearMonth { .. } => to_pyearmonth(e, v),
        TempData::PMonthDay { .. } => to_pmonthday(e, v),
        TempData::Duration { .. } => {
            let f = to_duration(e, v)?;
            Ok(TempData::Duration { f })
        }
    }
}

fn to_pyearmonth(e: &mut Engine, v: Value) -> JsResult<TempData> {
    match v {
        Value::Obj(id) => match e.heap.obj(id).map(|o| &o.kind) {
            Some(crate::heap::ObjKind::Temp(t)) => {
                let r = match *t {
                    x @ TempData::PYearMonth { .. } => x,
                    TempData::PDate { y, m, .. } | TempData::PDateTime { y, m, .. } => {
                        TempData::PYearMonth { y, m }
                    }
                    _ => return Err(e.type_error("PlainYearMonth expected")),
                };
                Ok(r)
            }
            Some(_) => {
                let y = field(e, id, "year")?.ok_or_else(|| e.type_error("year required"))?;
                let m = field(e, id, "month")?.unwrap_or(1) as u8;
                if !(1..=12).contains(&m) {
                    return Err(e.range_error("invalid month"));
                }
                Ok(TempData::PYearMonth { y, m })
            }
            None => Err(e.type_error("PlainYearMonth expected")),
        },
        Value::Str(id) => {
            let s = e.heap.string(id).unwrap_or_default();
            match parse_date_part(s.split(['T', ' ']).next().unwrap_or(&s)) {
                Some((y, m, _d)) if (1..=12).contains(&m) => Ok(TempData::PYearMonth { y, m }),
                _ => Err(e.range_error("invalid PlainYearMonth string")),
            }
        }
        _ => Err(e.type_error("PlainYearMonth expected")),
    }
}

fn to_pmonthday(e: &mut Engine, v: Value) -> JsResult<TempData> {
    match v {
        Value::Obj(id) => match e.heap.obj(id).map(|o| &o.kind) {
            Some(crate::heap::ObjKind::Temp(t)) => {
                let r = match *t {
                    x @ TempData::PMonthDay { .. } => x,
                    TempData::PDate { m, d, .. } | TempData::PDateTime { m, d, .. } => {
                        TempData::PMonthDay { m, d }
                    }
                    _ => return Err(e.type_error("PlainMonthDay expected")),
                };
                Ok(r)
            }
            Some(_) => {
                let mc = match e.get_prop(id, "monthCode") {
                    Some(Value::Str(sid)) => {
                        let s = e.heap.string(sid).unwrap_or_default();
                        s.strip_prefix('M').and_then(|x| x.parse::<u8>().ok())
                    }
                    _ => field(e, id, "month")?.map(|x| x as u8),
                };
                let d = field(e, id, "day")?;
                match (mc, d) {
                    (Some(m), Some(d)) => {
                        let (m, d) = (m, d as u8);
                        if !(1..=12).contains(&m) || d < 1 || d > dim(2000, m) {
                            return Err(e.range_error("invalid month-day"));
                        }
                        Ok(TempData::PMonthDay { m, d })
                    }
                    _ => Err(e.type_error("PlainMonthDay fields missing")),
                }
            }
            None => Err(e.type_error("PlainMonthDay expected")),
        },
        Value::Str(id) => {
            let s = e.heap.string(id).unwrap_or_default();
            // "MM-DD" ou date complète (on prend le mois/jour).
            let s = s.trim_start_matches(['T', ' ']);
            let (m, d) = if s.len() >= 5
                && s.as_bytes()[2] == b'-'
                && s[..2].starts_with(|c: char| c.is_ascii_digit())
                && !s[..1].contains('+')
                && s[..4].parse::<i64>().is_err()
            {
                (s[..2].parse().ok(), s[3..5].parse().ok())
            } else if let Some((_, m, d)) = parse_date_part(s) {
                (Some(m), Some(d))
            } else {
                (None, None)
            };
            match (m, d) {
                (Some(m), Some(d)) if (1..=12).contains(&m) && (1..=31).contains(&d) => {
                    Ok(TempData::PMonthDay { m, d })
                }
                _ => Err(e.range_error("invalid PlainMonthDay string")),
            }
        }
        _ => Err(e.type_error("PlainMonthDay expected")),
    }
}

/// Ordre canonique entre deux `TempData` de même famille.
fn temporal_cmp(a: &TempData, b: &TempData) -> i32 {
    let key = |t: &TempData| -> i128 {
        match t {
            TempData::Instant { ns } => *ns,
            TempData::PDate { y, m, d } | TempData::PDateTime { y, m, d, .. } => {
                let base = match t {
                    TempData::PDateTime {
                        h,
                        mi,
                        s_,
                        ms,
                        us,
                        ns,
                        ..
                    } => {
                        i128::from(*h) * 3_600_000_000_000
                            + i128::from(*mi) * 60_000_000_000
                            + i128::from(*s_) * 1_000_000_000
                            + i128::from(*ms) * 1_000_000
                            + i128::from(*us) * 1_000
                            + i128::from(*ns)
                    }
                    _ => 0,
                };
                i128::from(days_from_civil(*y, i64::from(*m), i64::from(*d))) * 86_400_000_000_000
                    + base
            }
            TempData::PTime {
                h,
                mi,
                s_,
                ms,
                us,
                ns,
            } => {
                i128::from(*h) * 3_600_000_000_000
                    + i128::from(*mi) * 60_000_000_000
                    + i128::from(*s_) * 1_000_000_000
                    + i128::from(*ms) * 1_000_000
                    + i128::from(*us) * 1_000
                    + i128::from(*ns)
            }
            TempData::PYearMonth { y, m } => i128::from(*y * 12 + i64::from(*m)),
            TempData::PMonthDay { m, d } => i128::from(*m) * 100 + i128::from(*d),
            TempData::Duration { .. } => 0, // cf. duration_compare
        }
    };
    let (x, y) = (key(a), key(b));
    x.cmp(&y) as i32
}

// -- add / subtract ------------------------------------------------------

fn t_add(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    temp_add(e, this, args, false)
}

fn t_subtract(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    temp_add(e, this, args, true)
}

fn temp_add(e: &mut Engine, this: Value, args: &[Value], neg: bool) -> JsResult<Value> {
    let (_, t) = td_id(e, this)?;
    let mut f = to_duration(e, arg(args, 0))?;
    if neg {
        for x in &mut f {
            *x = -*x;
        }
    }
    match t {
        TempData::Instant { ns } => {
            if f[..4].iter().any(|&x| x != 0) {
                return Err(
                    e.range_error("Instant arithmetic requires a duration without date units")
                );
            }
            mk(
                e,
                "Instant",
                TempData::Instant {
                    ns: ns + dur_time_ns(&f),
                },
            )
        }
        TempData::PDate { y, m, d } => {
            // Champs sub-jour ignorés (déviation documentée).
            let (ny, nm, nd) = add_ymd(y, m, d, f[0], f[1], f[2] * 7 + f[3]);
            if !valid_date(ny, nm, nd) {
                return Err(e.range_error("date out of range"));
            }
            mk(
                e,
                "PlainDate",
                TempData::PDate {
                    y: ny,
                    m: nm,
                    d: nd,
                },
            )
        }
        TempData::PTime {
            h,
            mi,
            s_,
            ms,
            us,
            ns,
        } => {
            let (_, (h2, m2, s2, ms2, us2, ns2)) = add_time(h, mi, s_, ms, us, ns, dur_time_ns(&f));
            mk(
                e,
                "PlainTime",
                TempData::PTime {
                    h: h2,
                    mi: m2,
                    s_: s2,
                    ms: ms2,
                    us: us2,
                    ns: ns2,
                },
            )
        }
        TempData::PDateTime {
            y,
            m,
            d,
            h,
            mi,
            s_,
            ms,
            us,
            ns,
        } => {
            let (carry, (h2, m2, s2, ms2, us2, ns2)) =
                add_time(h, mi, s_, ms, us, ns, dur_time_ns(&f));
            let (ny, nm, nd) = add_ymd(y, m, d, f[0], f[1], f[2] * 7 + f[3] + carry);
            if !valid_date(ny, nm, nd) {
                return Err(e.range_error("datetime out of range"));
            }
            mk(
                e,
                "PlainDateTime",
                TempData::PDateTime {
                    y: ny,
                    m: nm,
                    d: nd,
                    h: h2,
                    mi: m2,
                    s_: s2,
                    ms: ms2,
                    us: us2,
                    ns: ns2,
                },
            )
        }
        TempData::PYearMonth { y, m } => {
            // Jours de la durée ignorés (spec utiliserait refDay).
            let (ny, nm, _) = add_ymd(y, m, 1, f[0], f[1], 0);
            mk(e, "PlainYearMonth", TempData::PYearMonth { y: ny, m: nm })
        }
        TempData::Duration { f: a } => {
            let mut r = [0i64; 10];
            for i in 0..10 {
                r[i] = a[i] + f[i];
            }
            if r.iter().any(|&x| x > 0) && r.iter().any(|&x| x < 0) {
                return Err(e.range_error("duration fields would have mixed signs"));
            }
            mk(e, "Duration", TempData::Duration { f: r })
        }
        TempData::PMonthDay { .. } => Err(e.type_error("PlainMonthDay has no add/subtract")),
    }
}

// -- since / until --------------------------------------------------------

fn t_since(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    temp_diff(e, this, args)
}

fn t_until(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    temp_diff(e, this, args)
}

fn temp_diff(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let (_, t) = td_id(e, this)?;
    let o = to_same(e, &t, arg(args, 0))?;
    let lu = largest_unit(e, arg(args, 1), "")?;
    let f = diff_fields(e, &t, &o, &lu)?;
    mk(e, "Duration", TempData::Duration { f })
}

/// Différence `other − this` en champs de durée selon
/// `largestUnit` (défauts spec par type).
fn diff_fields(e: &mut Engine, a: &TempData, b: &TempData, lu: &str) -> JsResult<[i64; 10]> {
    let mut f = [0i64; 10];
    match (a, b) {
        (TempData::Instant { ns: x }, TempData::Instant { ns: y }) => {
            let lu = if lu.is_empty() { "seconds" } else { lu };
            if matches!(lu, "days" | "weeks" | "months" | "years" | "auto") {
                return Err(e.range_error("Instant largestUnit must be a time unit (≤ hours)"));
            }
            f = ns_to_dur(y - x, lu);
        }
        (TempData::PTime { .. }, TempData::PTime { .. }) => {
            let lu = if lu.is_empty() { "hours" } else { lu };
            let (x, y) = (time_ns(a), time_ns(b));
            if matches!(lu, "days" | "weeks" | "months" | "years") {
                return Err(e.range_error("PlainTime largestUnit must be a time unit"));
            }
            f = ns_to_dur(y - x, lu);
        }
        (
            TempData::PDate {
                y: y1,
                m: m1,
                d: d1,
            },
            TempData::PDate {
                y: y2,
                m: m2,
                d: d2,
            },
        ) => {
            let lu = if lu.is_empty() { "days" } else { lu };
            let (yy, mm, dd) = diff_date(
                *y1,
                *m1,
                *d1,
                *y2,
                *m2,
                *d2,
                lu != "auto" && matches!(lu, "years" | "months" | "year" | "month"),
            );
            match lu {
                "days" | "auto" | "" => {
                    f[3] = days_from_civil(*y2, i64::from(*m2), i64::from(*d2))
                        - days_from_civil(*y1, i64::from(*m1), i64::from(*d1));
                }
                "weeks" | "week" => {
                    let t = days_from_civil(*y2, i64::from(*m2), i64::from(*d2))
                        - days_from_civil(*y1, i64::from(*m1), i64::from(*d1));
                    f[2] = t / 7;
                    f[3] = t % 7;
                }
                "months" | "month" => {
                    f[1] = yy * 12 + mm;
                    f[3] = dd;
                }
                "years" | "year" => {
                    f[0] = yy;
                    f[1] = mm;
                    f[3] = dd;
                }
                _ => return Err(e.range_error("invalid largestUnit")),
            }
        }
        (
            TempData::PDateTime {
                y: y1,
                m: m1,
                d: d1,
                ..
            },
            TempData::PDateTime {
                y: y2,
                m: m2,
                d: d2,
                ..
            },
        ) => {
            let lu = if lu.is_empty() { "days" } else { lu };
            let t1 = time_ns(a);
            let t2 = time_ns(b);
            let days1 = days_from_civil(*y1, i64::from(*m1), i64::from(*d1));
            let days2 = days_from_civil(*y2, i64::from(*m2), i64::from(*d2));
            let total_ns = i128::from(days2 - days1) * 86_400_000_000_000 + (t2 - t1);
            match lu {
                "days" | "auto" | "" => {
                    f[3] = (total_ns / 86_400_000_000_000) as i64;
                    let rem = total_ns % 86_400_000_000_000;
                    let tf = ns_to_dur(rem, "hours");
                    f[4..].copy_from_slice(&tf[4..]);
                }
                "weeks" => {
                    f[2] = (total_ns / 86_400_000_000_000) as i64 / 7;
                    let rem = total_ns - i128::from(f[2] * 7) * 86_400_000_000_000;
                    f[3] = (rem / 86_400_000_000_000) as i64;
                    let tf = ns_to_dur(rem % 86_400_000_000_000, "hours");
                    f[4..].copy_from_slice(&tf[4..]);
                }
                "months" | "years" => {
                    let (yy, mm, dd) = diff_date(*y1, *m1, *d1, *y2, *m2, *d2, true);
                    if lu == "years" {
                        f[0] = yy;
                        f[1] = mm;
                    } else {
                        f[1] = yy * 12 + mm;
                    }
                    let rem = i128::from(dd) * 86_400_000_000_000 + (t2 - t1);
                    f[3] = (rem / 86_400_000_000_000) as i64;
                    let tf = ns_to_dur(rem % 86_400_000_000_000, "hours");
                    f[4..].copy_from_slice(&tf[4..]);
                }
                _ => {
                    // Plus petit que le jour : tout en temps.
                    f = ns_to_dur(total_ns, lu);
                }
            }
        }
        (TempData::PYearMonth { y: y1, m: m1 }, TempData::PYearMonth { y: y2, m: m2 }) => {
            let lu = if lu.is_empty() { "months" } else { lu };
            let months = (y2 - y1) * 12 + i64::from(*m2) - i64::from(*m1);
            match lu {
                "months" | "auto" | "" => f[1] = months,
                "years" => {
                    f[0] = months / 12;
                    f[1] = months % 12;
                }
                _ => {
                    return Err(e.range_error("PlainYearMonth largestUnit must be months or years"))
                }
            }
        }
        (TempData::Duration { f: x }, TempData::Duration { f: y }) => {
            for i in 0..10 {
                f[i] = y[i] - x[i];
            }
        }
        _ => return Err(e.type_error("cannot mix Temporal types")),
    }
    Ok(f)
}

fn time_ns(t: &TempData) -> i128 {
    match t {
        TempData::PTime {
            h,
            mi,
            s_,
            ms,
            us,
            ns,
        }
        | TempData::PDateTime {
            h,
            mi,
            s_,
            ms,
            us,
            ns,
            ..
        } => {
            i128::from(*h) * 3_600_000_000_000
                + i128::from(*mi) * 60_000_000_000
                + i128::from(*s_) * 1_000_000_000
                + i128::from(*ms) * 1_000_000
                + i128::from(*us) * 1_000
                + i128::from(*ns)
        }
        _ => 0,
    }
}

// -- with / conversions ---------------------------------------------------

fn t_with(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let (_, t) = td_id(e, this)?;
    let Value::Obj(o) = arg(args, 0) else {
        return Err(e.type_error("with expects an object"));
    };
    match t {
        TempData::PDate { y, m, d } => {
            let y = field(e, o, "year")?.unwrap_or(y);
            let m = field(e, o, "month")?.unwrap_or(i64::from(m)) as u8;
            let d = field(e, o, "day")?.unwrap_or(i64::from(d)) as u8;
            if !valid_date(y, m, d) {
                return Err(e.range_error("invalid date"));
            }
            mk(e, "PlainDate", TempData::PDate { y, m, d })
        }
        TempData::PTime {
            h,
            mi,
            s_,
            ms,
            us,
            ns,
        } => {
            let h = field(e, o, "hour")?.unwrap_or(i64::from(h)) as u8;
            let mi = field(e, o, "minute")?.unwrap_or(i64::from(mi)) as u8;
            let s_ = field(e, o, "second")?.unwrap_or(i64::from(s_)) as u8;
            let ms = field(e, o, "millisecond")?.unwrap_or(i64::from(ms)) as u16;
            let us = field(e, o, "microsecond")?.unwrap_or(i64::from(us)) as u16;
            let ns = field(e, o, "nanosecond")?.unwrap_or(i64::from(ns)) as u16;
            if h > 23 || mi > 59 || s_ > 59 || ms > 999 || us > 999 || ns > 999 {
                return Err(e.range_error("invalid time"));
            }
            mk(
                e,
                "PlainTime",
                TempData::PTime {
                    h,
                    mi,
                    s_,
                    ms,
                    us,
                    ns,
                },
            )
        }
        TempData::PDateTime {
            y,
            m,
            d,
            h,
            mi,
            s_,
            ms,
            us,
            ns,
        } => {
            let y = field(e, o, "year")?.unwrap_or(y);
            let m = field(e, o, "month")?.unwrap_or(i64::from(m)) as u8;
            let d = field(e, o, "day")?.unwrap_or(i64::from(d)) as u8;
            let h = field(e, o, "hour")?.unwrap_or(i64::from(h)) as u8;
            let mi = field(e, o, "minute")?.unwrap_or(i64::from(mi)) as u8;
            let s_ = field(e, o, "second")?.unwrap_or(i64::from(s_)) as u8;
            let ms = field(e, o, "millisecond")?.unwrap_or(i64::from(ms)) as u16;
            let us = field(e, o, "microsecond")?.unwrap_or(i64::from(us)) as u16;
            let ns = field(e, o, "nanosecond")?.unwrap_or(i64::from(ns)) as u16;
            if !valid_date(y, m, d)
                || h > 23
                || mi > 59
                || s_ > 59
                || ms > 999
                || us > 999
                || ns > 999
            {
                return Err(e.range_error("invalid datetime"));
            }
            mk(
                e,
                "PlainDateTime",
                TempData::PDateTime {
                    y,
                    m,
                    d,
                    h,
                    mi,
                    s_,
                    ms,
                    us,
                    ns,
                },
            )
        }
        TempData::PYearMonth { y, m } => {
            let y = field(e, o, "year")?.unwrap_or(y);
            let m = field(e, o, "month")?.unwrap_or(i64::from(m)) as u8;
            if !(1..=12).contains(&m) {
                return Err(e.range_error("invalid month"));
            }
            mk(e, "PlainYearMonth", TempData::PYearMonth { y, m })
        }
        TempData::PMonthDay { m, d } => {
            let m = field(e, o, "month")?.unwrap_or(i64::from(m)) as u8;
            let d = field(e, o, "day")?.unwrap_or(i64::from(d)) as u8;
            if !(1..=12).contains(&m) || d < 1 || d > dim(2000, m) {
                return Err(e.range_error("invalid month-day"));
            }
            mk(e, "PlainMonthDay", TempData::PMonthDay { m, d })
        }
        TempData::Duration { f: a } => {
            let mut r = a;
            for (i, k) in [
                "years",
                "months",
                "weeks",
                "days",
                "hours",
                "minutes",
                "seconds",
                "milliseconds",
                "microseconds",
                "nanoseconds",
            ]
            .iter()
            .enumerate()
            {
                if let Some(x) = field(e, o, k)? {
                    r[i] = x;
                }
            }
            if r.iter().any(|&x| x > 0) && r.iter().any(|&x| x < 0) {
                return Err(e.range_error("duration fields must have the same sign"));
            }
            mk(e, "Duration", TempData::Duration { f: r })
        }
        TempData::Instant { .. } => Err(e.type_error("Instant has no with()")),
    }
}

fn t_to_plain_date_time(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let (_, t) = td_id(e, this)?;
    let TempData::PDate { y, m, d } = t else {
        return Err(e.type_error("PlainDate expected"));
    };
    let (h, mi, s_, ms, us, ns) = match arg(args, 0) {
        Value::Undefined => (0, 0, 0, 0, 0, 0),
        v => {
            let pt = to_ptime(e, v)?;
            match pt {
                TempData::PTime {
                    h,
                    mi,
                    s_,
                    ms,
                    us,
                    ns,
                } => (h, mi, s_, ms, us, ns),
                _ => (0, 0, 0, 0, 0, 0),
            }
        }
    };
    mk(
        e,
        "PlainDateTime",
        TempData::PDateTime {
            y,
            m,
            d,
            h,
            mi,
            s_,
            ms,
            us,
            ns,
        },
    )
}

fn t_to_plain_date(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let (_, t) = td_id(e, this)?;
    match t {
        TempData::PDateTime { y, m, d, .. } => mk(e, "PlainDate", TempData::PDate { y, m, d }),
        TempData::PYearMonth { y, m } => {
            let d = int_arg_def(e, args, 0, 1, "day")? as u8;
            if d < 1 || d > dim(y, m) {
                return Err(e.range_error("invalid day"));
            }
            mk(e, "PlainDate", TempData::PDate { y, m, d })
        }
        TempData::PMonthDay { m, d } => {
            let y = int_arg(e, arg(args, 0), "year")?;
            if !valid_date(y, m, d) {
                return Err(e.range_error("invalid date"));
            }
            mk(e, "PlainDate", TempData::PDate { y, m, d })
        }
        _ => Err(e.type_error("toPlainDate on incompatible receiver")),
    }
}

fn t_to_plain_time(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    let (_, t) = td_id(e, this)?;
    match t {
        TempData::PDateTime {
            h,
            mi,
            s_,
            ms,
            us,
            ns,
            ..
        } => mk(
            e,
            "PlainTime",
            TempData::PTime {
                h,
                mi,
                s_,
                ms,
                us,
                ns,
            },
        ),
        _ => Err(e.type_error("toPlainTime on incompatible receiver")),
    }
}

fn t_to_ym(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    let (_, t) = td_id(e, this)?;
    match t {
        TempData::PDate { y, m, .. } | TempData::PDateTime { y, m, .. } => {
            mk(e, "PlainYearMonth", TempData::PYearMonth { y, m })
        }
        _ => Err(e.type_error("toPlainYearMonth on incompatible receiver")),
    }
}

fn t_to_md(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    let (_, t) = td_id(e, this)?;
    match t {
        TempData::PDate { m, d, .. } | TempData::PDateTime { m, d, .. } => {
            mk(e, "PlainMonthDay", TempData::PMonthDay { m, d })
        }
        _ => Err(e.type_error("toPlainMonthDay on incompatible receiver")),
    }
}

// -- Duration extras --------------------------------------------------------

fn t_neg(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    let (_, t) = td_id(e, this)?;
    match t {
        TempData::Duration { mut f } => {
            for x in &mut f {
                *x = -*x;
            }
            mk(e, "Duration", TempData::Duration { f })
        }
        _ => Err(e.type_error("Duration expected")),
    }
}

fn t_abs(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    let (_, t) = td_id(e, this)?;
    match t {
        TempData::Duration { mut f } => {
            for x in &mut f {
                *x = x.abs();
            }
            mk(e, "Duration", TempData::Duration { f })
        }
        _ => Err(e.type_error("Duration expected")),
    }
}

/// `Duration.prototype.total({unit})` — jours = 24 h, semaines =
/// 7 j ; mois/années non nuls sans `relativeTo` → `RangeError`
/// (spec).
fn t_total(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let (_, t) = td_id(e, this)?;
    let TempData::Duration { f } = t else {
        return Err(e.type_error("Duration expected"));
    };
    let unit = match arg(args, 0) {
        Value::Obj(id) => match e.get_prop(id, "unit") {
            Some(v) => e.to_js_string(v)?,
            None => return Err(e.type_error("total needs {unit}")),
        },
        v => e.to_js_string(v)?,
    };
    if f[0] != 0 || f[1] != 0 {
        return Err(e.range_error("total requires relativeTo for months/years"));
    }
    let days = f[2] * 7 + f[3];
    let ns = i128::from(days) * 86_400_000_000_000 + dur_time_ns(&f);
    let div: i128 = match unit.as_str() {
        "days" | "day" => 86_400_000_000_000,
        "hours" | "hour" => 3_600_000_000_000,
        "minutes" | "minute" => 60_000_000_000,
        "seconds" | "second" => 1_000_000_000,
        "milliseconds" | "millisecond" => 1_000_000,
        "microseconds" | "microsecond" => 1_000,
        "nanoseconds" | "nanosecond" => 1,
        "weeks" | "week" => 604_800_000_000_000,
        _ => return Err(e.range_error("invalid total unit")),
    };
    Ok(Value::Num(ns as f64 / div as f64))
}

/// `Duration.compare(a, b, {relativeTo})` — champs date non nuls
/// exigent `relativeTo` (spec) : on l'évalue comme `PlainDate`.
fn duration_compare(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let a = to_duration(e, arg(args, 0))?;
    let b = to_duration(e, arg(args, 1))?;
    let rel = relative_to(e, arg(args, 2))?;
    let total = |f: &[i64; 10], e: &mut Engine| -> JsResult<i128> {
        let date_part = f[..4].iter().any(|&x| x != 0);
        let base = dur_time_ns(f);
        if !date_part {
            return Ok(base);
        }
        let Some((y, m, d)) = rel else {
            return Err(e.range_error("Duration.compare requires relativeTo for date units"));
        };
        let (y2, m2, d2) = add_ymd(y, m, d, f[0], f[1], f[2] * 7 + f[3]);
        Ok(i128::from(
            days_from_civil(y2, i64::from(m2), i64::from(d2))
                - days_from_civil(y, i64::from(m), i64::from(d)),
        ) * 86_400_000_000_000
            + base)
    };
    let (x, y) = (total(&a, e)?, total(&b, e)?);
    Ok(Value::Num(x.cmp(&y) as i32 as f64))
}

// -- constructeurs / statiques ----------------------------------------------

fn instant_ctor(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    // Spec : `new Instant(epochNanoseconds)` — BigInt requis.
    match arg(args, 0) {
        Value::Big(_) => {
            let b = e.to_bigint(arg(args, 0))?;
            let ns = b
                .to_i128()
                .ok_or_else(|| e.range_error("epochNanoseconds out of range"))?;
            mk(e, "Instant", TempData::Instant { ns })
        }
        _ => Err(e.type_error("Instant expects a BigInt nanosecond value")),
    }
}

fn instant_from(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let t = to_instant(e, arg(args, 0))?;
    mk(e, "Instant", t)
}

fn instant_from_ms(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let ms = int_arg(e, arg(args, 0), "epochMilliseconds")?;
    mk(
        e,
        "Instant",
        TempData::Instant {
            ns: i128::from(ms) * 1_000_000,
        },
    )
}

fn instant_from_ns(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    match arg(args, 0) {
        Value::Big(_) => {
            let b = e.to_bigint(arg(args, 0))?;
            let ns = b
                .to_i128()
                .ok_or_else(|| e.range_error("epochNanoseconds out of range"))?;
            mk(e, "Instant", TempData::Instant { ns })
        }
        _ => Err(e.type_error("fromEpochNanoseconds expects a BigInt")),
    }
}

fn instant_compare(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let a = to_instant(e, arg(args, 0))?;
    let b = to_instant(e, arg(args, 1))?;
    Ok(Value::Num(temporal_cmp(&a, &b) as f64))
}

fn pdate_ctor(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let y = int_arg(e, arg(args, 0), "year")?;
    let m = int_arg(e, arg(args, 1), "month")? as u8;
    let d = int_arg(e, arg(args, 2), "day")? as u8;
    if !valid_date(y, m, d) {
        return Err(e.range_error("invalid PlainDate"));
    }
    mk(e, "PlainDate", TempData::PDate { y, m, d })
}

fn pdate_from(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let t = to_pdate(e, arg(args, 0))?;
    mk(e, "PlainDate", t)
}

fn pdate_compare(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let a = to_pdate(e, arg(args, 0))?;
    let b = to_pdate(e, arg(args, 1))?;
    Ok(Value::Num(temporal_cmp(&a, &b) as f64))
}

fn ptime_ctor(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let h = int_arg_def(e, args, 0, 0, "hour")? as u8;
    let mi = int_arg_def(e, args, 1, 0, "minute")? as u8;
    let s_ = int_arg_def(e, args, 2, 0, "second")? as u8;
    let ms = int_arg_def(e, args, 3, 0, "millisecond")? as u16;
    let us = int_arg_def(e, args, 4, 0, "microsecond")? as u16;
    let ns = int_arg_def(e, args, 5, 0, "nanosecond")? as u16;
    if h > 23 || mi > 59 || s_ > 59 || ms > 999 || us > 999 || ns > 999 {
        return Err(e.range_error("invalid PlainTime"));
    }
    mk(
        e,
        "PlainTime",
        TempData::PTime {
            h,
            mi,
            s_,
            ms,
            us,
            ns,
        },
    )
}

fn ptime_from(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let t = to_ptime(e, arg(args, 0))?;
    mk(e, "PlainTime", t)
}

fn ptime_compare(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let a = to_ptime(e, arg(args, 0))?;
    let b = to_ptime(e, arg(args, 1))?;
    Ok(Value::Num(temporal_cmp(&a, &b) as f64))
}

fn pdt_ctor(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let y = int_arg(e, arg(args, 0), "year")?;
    let m = int_arg(e, arg(args, 1), "month")? as u8;
    let d = int_arg(e, arg(args, 2), "day")? as u8;
    let h = int_arg_def(e, args, 3, 0, "hour")? as u8;
    let mi = int_arg_def(e, args, 4, 0, "minute")? as u8;
    let s_ = int_arg_def(e, args, 5, 0, "second")? as u8;
    let ms = int_arg_def(e, args, 6, 0, "millisecond")? as u16;
    let us = int_arg_def(e, args, 7, 0, "microsecond")? as u16;
    let ns = int_arg_def(e, args, 8, 0, "nanosecond")? as u16;
    if !valid_date(y, m, d) || h > 23 || mi > 59 || s_ > 59 || ms > 999 || us > 999 || ns > 999 {
        return Err(e.range_error("invalid PlainDateTime"));
    }
    mk(
        e,
        "PlainDateTime",
        TempData::PDateTime {
            y,
            m,
            d,
            h,
            mi,
            s_,
            ms,
            us,
            ns,
        },
    )
}

fn pdt_from(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let t = to_pdatetime(e, arg(args, 0))?;
    mk(e, "PlainDateTime", t)
}

fn pdt_compare(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let a = to_pdatetime(e, arg(args, 0))?;
    let b = to_pdatetime(e, arg(args, 1))?;
    Ok(Value::Num(temporal_cmp(&a, &b) as f64))
}

fn pym_ctor(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let y = int_arg(e, arg(args, 0), "year")?;
    let m = int_arg(e, arg(args, 1), "month")? as u8;
    if !(1..=12).contains(&m) {
        return Err(e.range_error("invalid PlainYearMonth"));
    }
    mk(e, "PlainYearMonth", TempData::PYearMonth { y, m })
}

fn pym_from(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let t = to_pyearmonth(e, arg(args, 0))?;
    mk(e, "PlainYearMonth", t)
}

fn pym_compare(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let a = to_pyearmonth(e, arg(args, 0))?;
    let b = to_pyearmonth(e, arg(args, 1))?;
    Ok(Value::Num(temporal_cmp(&a, &b) as f64))
}

fn pmd_ctor(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let m = int_arg(e, arg(args, 0), "month")? as u8;
    let d = int_arg(e, arg(args, 1), "day")? as u8;
    if !(1..=12).contains(&m) || d < 1 || d > dim(2000, m) {
        return Err(e.range_error("invalid PlainMonthDay"));
    }
    mk(e, "PlainMonthDay", TempData::PMonthDay { m, d })
}

fn pmd_from(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let t = to_pmonthday(e, arg(args, 0))?;
    mk(e, "PlainMonthDay", t)
}

fn pmd_compare(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let a = to_pmonthday(e, arg(args, 0))?;
    let b = to_pmonthday(e, arg(args, 1))?;
    Ok(Value::Num(temporal_cmp(&a, &b) as f64))
}

fn duration_ctor(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let mut f = [0i64; 10];
    for (i, k) in [
        "years",
        "months",
        "weeks",
        "days",
        "hours",
        "minutes",
        "seconds",
        "milliseconds",
        "microseconds",
        "nanoseconds",
    ]
    .iter()
    .enumerate()
    {
        f[i] = int_arg_def(e, args, i, 0, k)?;
    }
    if f.iter().any(|&x| x > 0) && f.iter().any(|&x| x < 0) {
        return Err(e.range_error("duration fields must have the same sign"));
    }
    mk(e, "Duration", TempData::Duration { f })
}

fn duration_from(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let f = to_duration(e, arg(args, 0))?;
    mk(e, "Duration", TempData::Duration { f })
}

// -- Temporal.Now ------------------------------------------------------------

fn now_ns() -> i128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos() as i128)
}

fn now_instant(e: &mut Engine, _t: Value, _a: &[Value]) -> JsResult<Value> {
    mk(e, "Instant", TempData::Instant { ns: now_ns() })
}

fn now_pdt(e: &mut Engine, _t: Value, _a: &[Value]) -> JsResult<Value> {
    let ms = (now_ns() / 1_000_000) as f64;
    let (y, m, d, _w, h, mi, s, mss) = crate::builtins::ms_to_parts(ms);
    mk(
        e,
        "PlainDateTime",
        TempData::PDateTime {
            y,
            m: m as u8,
            d: d as u8,
            h: h as u8,
            mi: mi as u8,
            s_: s as u8,
            ms: mss as u16,
            us: 0,
            ns: 0,
        },
    )
}

fn now_pdate(e: &mut Engine, _t: Value, _a: &[Value]) -> JsResult<Value> {
    let ms = (now_ns() / 1_000_000) as f64;
    let (y, m, d, _w, _h, _mi, _s, _mss) = crate::builtins::ms_to_parts(ms);
    mk(
        e,
        "PlainDate",
        TempData::PDate {
            y,
            m: m as u8,
            d: d as u8,
        },
    )
}

fn now_ptime(e: &mut Engine, _t: Value, _a: &[Value]) -> JsResult<Value> {
    let ms = (now_ns() / 1_000_000) as f64;
    let (_y, _m, _d, _w, h, mi, s, mss) = crate::builtins::ms_to_parts(ms);
    mk(
        e,
        "PlainTime",
        TempData::PTime {
            h: h as u8,
            mi: mi as u8,
            s_: s as u8,
            ms: mss as u16,
            us: 0,
            ns: 0,
        },
    )
}

fn now_tz(e: &mut Engine, _t: Value, _a: &[Value]) -> JsResult<Value> {
    Ok(e.str_val("UTC"))
}

// -- install -------------------------------------------------------------------

/// `(méthode → fn)` communes aux protos date/heure.
const COMMON: &[(&str, NativeFn)] = &[
    ("toString", t_to_string),
    ("toJSON", t_to_json),
    ("valueOf", t_value_of),
    ("equals", t_equals),
    ("with", t_with),
];

/// `add`/`subtract`/`since`/`until` partagés.
const ARITH: &[(&str, NativeFn)] = &[
    ("add", t_add),
    ("subtract", t_subtract),
    ("since", t_since),
    ("until", t_until),
];

fn methods(e: &mut Engine, proto: ObjId, list: &[(&'static str, NativeFn)]) {
    for (n, f) in list {
        e.add_method(proto, n, *f);
    }
}

fn tag(e: &mut Engine, proto: ObjId, name: &str) {
    let t = e.str_val(format!("Temporal.{name}"));
    e.set_own(proto, crate::interp::SYM_TO_STRING_TAG, t, false);
}

/// Installe `Temporal` sur l'objet global.
pub(crate) fn install(e: &mut Engine) {
    let g = e.global;
    let t = e.new_object();

    // -- Instant ------------------------------------------------------------
    {
        let p = e.new_object();
        methods(e, p, COMMON);
        methods(e, p, ARITH);
        tag(e, p, "Instant");
        let c = ctor(e, "Instant", instant_ctor, p);
        if let Value::Obj(cid) = c {
            e.add_method(cid, "from", instant_from);
            e.add_method(cid, "fromEpochMilliseconds", instant_from_ms);
            e.add_method(cid, "fromEpochNanoseconds", instant_from_ns);
            e.add_method(cid, "compare", instant_compare);
        }
        e.set_own(t, "Instant", c, false);
    }
    // -- PlainDate ----------------------------------------------------------
    {
        let p = e.new_object();
        methods(e, p, COMMON);
        methods(e, p, ARITH);
        e.add_method(p, "toPlainDateTime", t_to_plain_date_time);
        e.add_method(p, "toPlainYearMonth", t_to_ym);
        e.add_method(p, "toPlainMonthDay", t_to_md);
        tag(e, p, "PlainDate");
        let c = ctor(e, "PlainDate", pdate_ctor, p);
        if let Value::Obj(cid) = c {
            e.add_method(cid, "from", pdate_from);
            e.add_method(cid, "compare", pdate_compare);
        }
        e.set_own(t, "PlainDate", c, false);
    }
    // -- PlainTime ----------------------------------------------------------
    {
        let p = e.new_object();
        methods(e, p, COMMON);
        methods(e, p, ARITH);
        tag(e, p, "PlainTime");
        let c = ctor(e, "PlainTime", ptime_ctor, p);
        if let Value::Obj(cid) = c {
            e.add_method(cid, "from", ptime_from);
            e.add_method(cid, "compare", ptime_compare);
        }
        e.set_own(t, "PlainTime", c, false);
    }
    // -- PlainDateTime ------------------------------------------------------
    {
        let p = e.new_object();
        methods(e, p, COMMON);
        methods(e, p, ARITH);
        e.add_method(p, "toPlainDate", t_to_plain_date);
        e.add_method(p, "toPlainTime", t_to_plain_time);
        e.add_method(p, "toPlainYearMonth", t_to_ym);
        e.add_method(p, "toPlainMonthDay", t_to_md);
        tag(e, p, "PlainDateTime");
        let c = ctor(e, "PlainDateTime", pdt_ctor, p);
        if let Value::Obj(cid) = c {
            e.add_method(cid, "from", pdt_from);
            e.add_method(cid, "compare", pdt_compare);
        }
        e.set_own(t, "PlainDateTime", c, false);
    }
    // -- PlainYearMonth -----------------------------------------------------
    {
        let p = e.new_object();
        methods(e, p, COMMON);
        methods(e, p, ARITH);
        e.add_method(p, "toPlainDate", t_to_plain_date);
        tag(e, p, "PlainYearMonth");
        let c = ctor(e, "PlainYearMonth", pym_ctor, p);
        if let Value::Obj(cid) = c {
            e.add_method(cid, "from", pym_from);
            e.add_method(cid, "compare", pym_compare);
        }
        e.set_own(t, "PlainYearMonth", c, false);
    }
    // -- PlainMonthDay ------------------------------------------------------
    {
        let p = e.new_object();
        methods(e, p, COMMON);
        e.add_method(p, "toPlainDate", t_to_plain_date);
        tag(e, p, "PlainMonthDay");
        let c = ctor(e, "PlainMonthDay", pmd_ctor, p);
        if let Value::Obj(cid) = c {
            e.add_method(cid, "from", pmd_from);
            e.add_method(cid, "compare", pmd_compare);
        }
        e.set_own(t, "PlainMonthDay", c, false);
    }
    // -- Duration -----------------------------------------------------------
    {
        let p = e.new_object();
        methods(e, p, COMMON);
        methods(e, p, ARITH);
        e.add_method(p, "neg", t_neg);
        e.add_method(p, "abs", t_abs);
        e.add_method(p, "total", t_total);
        tag(e, p, "Duration");
        let c = ctor(e, "Duration", duration_ctor, p);
        if let Value::Obj(cid) = c {
            e.add_method(cid, "from", duration_from);
            e.add_method(cid, "compare", duration_compare);
        }
        e.set_own(t, "Duration", c, false);
    }
    // -- Now -----------------------------------------------------------------
    {
        let now = e.new_object();
        e.add_method(now, "instant", now_instant);
        e.add_method(now, "plainDateTimeISO", now_pdt);
        e.add_method(now, "plainDateISO", now_pdate);
        e.add_method(now, "plainTimeISO", now_ptime);
        e.add_method(now, "timeZoneId", now_tz);
        let tg = e.str_val("Temporal.Now");
        e.set_own(now, crate::interp::SYM_TO_STRING_TAG, tg, false);
        e.set_own(t, "Now", Value::Obj(now), false);
    }

    let tg = e.str_val("Temporal");
    e.set_own(t, crate::interp::SYM_TO_STRING_TAG, tg, false);
    e.set_own(g, "Temporal", Value::Obj(t), false);

    // `c_id` importé par convention (ctor helper) — silence lint.
    let _ = c_id;
}
