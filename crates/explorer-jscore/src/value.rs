// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Core JS value representation and number formatting.

use std::cell::Cell;
use std::cmp::Ordering;
use std::ops::Deref;
use std::rc::Rc;

/// Handle into the `Heap` arena (ADR-0005-style:
/// objects reference each other by id, never by pointer).
pub type ObjId = u32;

/// Boîte d'une chaîne JS — JS-C420 : les chaînes sont sorties de
/// l'arène GC (elles produisaient 555+ Mio de cellules mortes
/// entre checkpoints — le sweep ne peut pas tourner mid-eval,
/// la pile Rust n'étant pas une racine). Le dernier `Rc`
/// déréférence exactement une fois la **capacité** du compteur
/// partagé `acct` (le `Drop` vit ici, pas sur [`JsStr`] — les
/// handles se clonent librement).
struct StrBox {
    /// Contenu UTF-8 — surrogates isolés rendus `U+FFFD`
    /// (couche affichage/DOM ; la vue exacte est `units`).
    s: String,
    /// Vue **UTF-16 exacte** — présente seulement quand la
    /// chaîne porte un surrogate isolé (fromCharCode, `\uXXXX`
    /// JSON/lexer, slice coupant une paire). `None` = `s`
    /// reflète les units 1:1 (chemin commun, zéro surcoût).
    units: Option<Vec<u16>>,
    /// Compteur d'octets vivants partagé avec l'`Engine`
    /// (`str_acct`) — incrémenté à l'alloc (avant
    /// l'emboîtement, pour que toute capacité comptée soit
    /// rendue), décrémenté à la mort du dernier handle.
    acct: Rc<Cell<isize>>,
}

impl Drop for StrBox {
    fn drop(&mut self) {
        let mut c = self.s.capacity() as isize;
        if let Some(u) = &self.units {
            c += (u.capacity() * 2) as isize;
        }
        self.acct.set(self.acct.get() - c);
    }
}

/// Chaîne JavaScript — `Rc` intra-`Value` : la mort est au
/// refcount, mid-eval, pas au prochain checkpoint GC. Clone =
/// bump de compteur (les temporaries d'éval partagent la même
/// boîte) ; `Deref → str` donne `len()`/`encode_utf16()`/…
/// sans clone.
#[derive(Clone)]
pub struct JsStr(Rc<StrBox>);

impl JsStr {
    /// Enrobe `s` en comptant sa **capacité** dans `acct`
    /// (cohérent avec l'ancien `cell_bytes` : `s += …` réserve
    /// amorti — compter `len` sous-estimait le RSS de moitié).
    pub fn new(s: impl Into<String>, acct: &Rc<Cell<isize>>) -> JsStr {
        let s = s.into();
        acct.set(acct.get() + s.capacity() as isize);
        crate::j17_tick(&crate::J17_STR_CELL);
        JsStr(Rc::new(StrBox {
            s,
            units: None,
            acct: acct.clone(),
        }))
    }

    /// Construit depuis des **code units UTF-16** (source
    /// `fromCharCode`, escape `\uXXXX`, slice, JSON) — garde les
    /// surrogates isolés dans `units` quand la conversion lossy
    /// les aurait perdus ; sinon tombe sur le stockage commun.
    pub(crate) fn from_units(units: Vec<u16>, acct: &Rc<Cell<isize>>) -> JsStr {
        let s = String::from_utf16_lossy(&units);
        let keep = units.iter().any(|u| (0xD800..0xE000).contains(u))
            && units != s.encode_utf16().collect::<Vec<u16>>().as_slice();
        let keep = if keep { Some(units) } else { None };
        let cap = s.capacity() + keep.as_ref().map_or(0, |u| u.capacity() * 2);
        acct.set(acct.get() + cap as isize);
        crate::j17_tick(&crate::J17_STR_CELL);
        JsStr(Rc::new(StrBox {
            s,
            units: keep,
            acct: acct.clone(),
        }))
    }

    /// Chaîne d'**un seul code unit UTF-16** (indexation `s[i]`,
    /// `charAt`) — surrogate isolé inclus.
    #[allow(dead_code)]
    pub(crate) fn from_unit(u: u16, acct: &Rc<Cell<isize>>) -> JsStr {
        Self::from_units(vec![u], acct)
    }

    /// Contenu UTF-8 emprunté — lecture zéro-clone (les
    /// anciens `heap.string(id)` clonaient l'intégralité).
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0.s
    }

    /// `Rc` partagé de la boîte — identité physique (cache
    /// `num_strs`, fast-path `===` sur même boîte).
    #[must_use]
    pub fn ptr_eq(a: &JsStr, b: &JsStr) -> bool {
        Rc::ptr_eq(&a.0, &b.0)
    }

    /// `Rc::strong_count` de la boîte — `1` = seul handle :
    /// mutation en place sûre ([`Self::try_append`]).
    #[must_use]
    pub fn strong_count(&self) -> usize {
        Rc::strong_count(&self.0)
    }

    /// Concaténation **en place** si ce handle est unique
    /// (`s += c` en boucle — l'ancien `format!("{ls}{rs}")`
    /// copiait l'opérande entière à chaque tour : O(n²) mort).
    /// Rend `false` (rien fait) si la boîte est partagée —
    /// l'appelant retombe sur l'alloc fraîche. La croissance
    /// de capacité est comptée dans `acct` pour que le `Drop`
    /// rende exactement ce qui fut pris.
    pub fn try_append(&mut self, rhs: &str) -> bool {
        let Some(b) = Rc::get_mut(&mut self.0) else {
            return false;
        };
        let old = b.s.capacity() + b.units.as_ref().map_or(0, |u| u.capacity() * 2);
        b.s.push_str(rhs);
        // `rhs` est UTF-8 valide — ses units sont lossless ;
        // on prolonge la vue exacte si elle existe.
        if let Some(u) = &mut b.units {
            u.extend(rhs.encode_utf16());
        }
        let new = b.s.capacity() + b.units.as_ref().map_or(0, |u| u.capacity() * 2);
        b.acct.set(b.acct.get() + (new - old) as isize);
        true
    }

    /// `try_append` avec une `JsStr` — préserve les surrogates :
    /// fusionne les vues `units` exactes (crée la nôtre si `rhs`
    /// en porte et pas nous).
    pub fn try_append_js(&mut self, rhs: &JsStr) -> bool {
        let Some(b) = Rc::get_mut(&mut self.0) else {
            return false;
        };
        let old = b.s.capacity() + b.units.as_ref().map_or(0, |u| u.capacity() * 2);
        let self_units: Vec<u16> = b
            .units
            .clone()
            .unwrap_or_else(|| b.s.encode_utf16().collect());
        b.s.push_str(rhs.as_str());
        if b.units.is_some() || rhs.0.units.is_some() {
            let mut u = self_units;
            u.extend(rhs.units_view());
            b.units = Some(u);
        }
        let new = b.s.capacity() + b.units.as_ref().map_or(0, |u| u.capacity() * 2);
        b.acct.set(b.acct.get() + new as isize - old as isize);
        true
    }

    /// `a + b` exact — units fusionnées si l'un des deux en
    /// porte (spec : les surrogates isolés traversent la
    /// concaténation).
    pub(crate) fn concat_js(a: &JsStr, b: &JsStr, acct: &Rc<Cell<isize>>) -> JsStr {
        if a.0.units.is_none() && b.0.units.is_none() {
            return JsStr::new(format!("{}{}", a.as_str(), b.as_str()), acct);
        }
        let mut u = a.units_view();
        u.extend(b.units_view());
        JsStr::from_units(u, acct)
    }

    /// Comparaison **par code units UTF-16** (spec `IsLessThan`
    /// sur les strings) — pas l'ordre UTF-8/code point : un
    /// astral (lead `0xD800..`) trie **avant** `U+E000..U+FFFF`
    /// en units mais après en code points (`'\uFFFF' <
    /// '\u{10000}'` est `false`, spec). Fast-path : deux chaînes
    /// sans `units` et sans char `≥ U+E000` (donc sans astral)
    /// ont le même ordre dans les deux vues.
    #[must_use]
    pub fn cmp_js(a: &JsStr, b: &JsStr) -> Ordering {
        if a.0.units.is_none()
            && b.0.units.is_none()
            && !a.as_str().chars().any(|c| (c as u32) >= 0xE000)
            && !b.as_str().chars().any(|c| (c as u32) >= 0xE000)
        {
            return a.as_str().cmp(b.as_str());
        }
        a.units_view().cmp(&b.units_view())
    }

    /// Vue UTF-16 matérialisée — `units` si présente, sinon
    /// `encode_utf16` de `s` (chemins u16 natifs : charCodeAt,
    /// JSON.stringify, slices, indexOf sur chaîne à surrogate).
    pub(crate) fn units_view(&self) -> Vec<u16> {
        match &self.0.units {
            Some(u) => u.clone(),
            None => self.as_str().encode_utf16().collect(),
        }
    }

    /// La boîte porte-t-elle une vue `units` exacte (surrogate
    /// isolé présent) — fast-path : `None` → `s` suffit
    /// partout.
    #[must_use]
    pub fn has_units(&self) -> bool {
        self.0.units.is_some()
    }

    /// `trim`/`trimStart`/`trimEnd` en units — whitespace JS =
    /// `White_Space` Unicode + `U+FEFF` (tous BMP ; un surrogate
    /// isolé n'est jamais whitespace et survit tel quel).
    #[must_use]
    pub fn trim_js(&self, start: bool, end: bool, acct: &Rc<Cell<isize>>) -> JsStr {
        let u = self.units_view();
        let mut lo = 0usize;
        let mut hi = u.len();
        if start {
            while lo < hi && unit_is_ws(u[lo]) {
                lo += 1;
            }
        }
        if end {
            while hi > lo && unit_is_ws(u[hi - 1]) {
                hi -= 1;
            }
        }
        self.slice_js_u16(lo, hi, acct)
    }

    /// Itération **par code point** (spec `@@iterator` /
    /// spread `[...s]`) : chaque item est la tranche units du
    /// code point — paire surrogate → 2 units (astral entier),
    /// surrogate **isolé** → 1 unit préservé (pas U+FFFD).
    pub(crate) fn iter_code_points(&self) -> Vec<Vec<u16>> {
        let u = self.units_view();
        let mut out = Vec::with_capacity(u.len());
        let mut i = 0;
        while i < u.len() {
            let w = u[i];
            let span = if (0xD800..0xDC00).contains(&w)
                && matches!(u.get(i + 1), Some(t) if (0xDC00..0xE000).contains(t))
            {
                2
            } else {
                1
            };
            out.push(u[i..i + span].to_vec());
            i += span;
        }
        out
    }

    /// Longueur en **code units UTF-16** — le `length` spec
    /// d'une Str (astral = 2, surrogate isolé = 1) sans la
    /// copie qu'un clone imposerait (§B-59).
    #[must_use]
    pub fn u16_len(&self) -> usize {
        match &self.0.units {
            Some(u) => u.len(),
            None => self.as_str().encode_utf16().count(),
        }
    }

    /// Code unit UTF-16 à l'indice `i` (`charCodeAt`/`at` en
    /// boucle sans clone de la chaîne entière, §B-59) —
    /// surrogate isolé rendu tel quel (spec).
    #[must_use]
    pub fn u16_at(&self, i: usize) -> Option<u16> {
        match &self.0.units {
            Some(u) => u.get(i).copied(),
            None => self.as_str().encode_utf16().nth(i),
        }
    }

    /// Offset octet du `u16_i`-ème code unit UTF-16 (frontière
    /// de char, ou `s.len()` au-delà) — conversion units→bytes
    /// sans matérialiser la vue (§B-59).
    pub(crate) fn u16_pos_to_byte(s: &str, u16_i: usize) -> usize {
        let mut u = 0usize;
        for (b, c) in s.char_indices() {
            if u >= u16_i {
                return b;
            }
            u += c.len_utf16();
        }
        s.len()
    }

    /// Recherche `needle` (vue **units**) — match **exact**
    /// code unit à code unit : un surrogate isolé n'égale
    /// jamais `U+FFFD` (spec `StringIndexOf`). `from_u16` borne
    /// le début ; `last` → dernière occurrence `≤`. Retour en
    /// **index de code unit** (le résultat spec), `-1` sinon.
    #[must_use]
    pub fn index_of_units(&self, needle: &[u16], from_u16: usize, last: bool) -> i64 {
        let u = self.units_view();
        if needle.is_empty() {
            return from_u16.min(u.len()) as i64;
        }
        if needle.len() > u.len() {
            return -1;
        }
        let max = u.len() - needle.len();
        if last {
            for i in (0..=from_u16.min(max)).rev() {
                if u[i..i + needle.len()] == needle[..] {
                    return i as i64;
                }
            }
        } else {
            if from_u16 > max {
                return -1;
            }
            for i in from_u16..=max {
                if u[i..i + needle.len()] == needle[..] {
                    return i as i64;
                }
            }
        }
        -1
    }

    /// `indexOf`/`lastIndexOf` en units UTF-16 — offset octet
    /// du `from_u16`-ième unit, `find`/`match_indices` sur la
    /// tranche, reconversion units ; `-1` si absent (§B-59).
    #[must_use]
    pub fn index_of(&self, needle: &str, from_u16: usize, last: bool) -> i64 {
        let s = self.as_str();
        let byte_from = Self::u16_pos_to_byte(s, from_u16);
        let pos = if needle.len() > s.len() {
            None
        } else if last {
            let limit = byte_from.min(s.len() - needle.len());
            s.match_indices(needle)
                .take_while(|(p, _)| *p <= limit)
                .last()
                .map(|(p, _)| p)
        } else if byte_from > s.len() - needle.len() {
            None
        } else {
            s[byte_from..].find(needle).map(|p| p + byte_from)
        };
        pos.map_or(-1, |b| s[..b].encode_utf16().count() as i64)
    }

    /// Tranche `[a, b)` en code units UTF-16 → `String` —
    /// usage texte interne/DOM (surrogate isolé → U+FFFD).
    /// Pour un résultat JS-visible, [`Self::slice_js_u16`].
    #[must_use]
    pub fn slice_u16(&self, a: usize, b: usize) -> String {
        let units: Vec<u16> = self
            .units_view()
            .into_iter()
            .skip(a)
            .take(b.saturating_sub(a))
            .collect();
        String::from_utf16_lossy(&units)
    }

    /// Tranche `[a, b)` en code units → `JsStr` **exacte** —
    /// `slice`/`substring`/`substr`/indexation JS : les
    /// surrogates isolés traversent (spec).
    pub(crate) fn slice_js_u16(&self, a: usize, b: usize, acct: &Rc<Cell<isize>>) -> JsStr {
        let units: Vec<u16> = self
            .units_view()
            .into_iter()
            .skip(a)
            .take(b.saturating_sub(a))
            .collect();
        JsStr::from_units(units, acct)
    }
}

impl Deref for JsStr {
    type Target = str;
    fn deref(&self) -> &str {
        self.as_str()
    }
}

impl AsRef<str> for JsStr {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

/// Égalité **de contenu** — la seule lecture spec pour `===`
/// sur chaînes (l'ancienne comparaison d'`ObjId` exigeait le
/// repli `str_eq` pour deux cellules au même texte).
impl PartialEq for JsStr {
    fn eq(&self, other: &Self) -> bool {
        match (&self.0.units, &other.0.units) {
            (None, None) => self.as_str() == other.as_str(),
            // 'a\uD800' !== 'a\uFFFD' : comparer les vues u16.
            _ => self.units_view() == other.units_view(),
        }
    }
}
impl Eq for JsStr {}

impl PartialEq<str> for JsStr {
    fn eq(&self, other: &str) -> bool {
        if self.0.units.is_some() {
            self.units_view() == other.encode_utf16().collect::<Vec<u16>>()
        } else {
            self.as_str() == other
        }
    }
}
impl PartialEq<&str> for JsStr {
    fn eq(&self, other: &&str) -> bool {
        *self == **other
    }
}

impl std::fmt::Debug for JsStr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self.as_str())
    }
}

impl std::fmt::Display for JsStr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A JavaScript value. `Clone` — les indirections `ObjId` sont
/// `Copy`, `JsStr` est un `Rc` (bump de compteur).
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    /// `undefined`
    Undefined,
    /// `null`
    Null,
    /// Boolean.
    Bool(bool),
    /// IEEE-754 double.
    Num(f64),
    /// Chaîne JS (`Rc` hors arène — JS-C420).
    Str(JsStr),
    /// `bigint` primitive — `ObjId` of an `ObjKind::BigInt` cell
    /// (primitive semantics : `1n === 1n`, never a `[[Prototype]]`
    /// chain — the id only carries the digits).
    Big(ObjId),
    /// Heap-allocated object.
    Obj(ObjId),
    /// Élément-trou de tableau épars (`Array(3)`, `delete a[i]`,
    /// `[1,,3]`). Jamais observable directement — `find_own` rend
    /// `None` (spec `HasProperty`) et les chemins génériques le
    /// traitent comme `undefined` ; les méthodes qui *sautent*
    /// les trous (`forEach`/`map`/…) testent `is_hole`.
    Hole,
}

// `Value` tient en 16 octets (`JsStr` = un `Rc` = 8 o, `Num` =
// 8 o — tag + payload alignés).
const _: () = assert!(std::mem::size_of::<Value>() <= 16);

impl Value {
    /// Whether the value is object-like (heap `Obj`).
    #[must_use]
    pub fn is_object(&self) -> bool {
        matches!(self, Value::Obj(_))
    }

    /// Trou de tableau épars.
    #[must_use]
    pub fn is_hole(&self) -> bool {
        matches!(self, Value::Hole)
    }

    /// `Hole → Undefined` — lecture d'un trou (itérateurs,
    /// valeurs retournées par `pop`/`at`/spread…).
    #[must_use]
    pub fn unhole(&self) -> Value {
        if self.is_hole() {
            Value::Undefined
        } else {
            self.clone()
        }
    }
}

/// `ToString(number)` — mirrors the JS rules: `NaN`, `Infinity`,
/// plain decimals, exponential form outside `[1e-6, 1e21)`.
pub fn num_to_js_string(n: f64) -> String {
    if n.is_nan() {
        return "NaN".into();
    }
    if n.is_infinite() {
        return if n < 0.0 { "-Infinity" } else { "Infinity" }.into();
    }
    if n == 0.0 {
        return "0".into();
    }
    let a = n.abs();
    if !(1e-6..1e21).contains(&a) {
        let s = format!("{n:e}");
        // Rust prints "1e21"/"1.5e-7"; JS wants "1e+21"/"1.5e-7".
        if let Some(i) = s.find('e') {
            let (mant, exp) = s.split_at(i + 1);
            let exp = &exp[0..];
            if exp.starts_with('-') {
                return format!("{mant}{exp}");
            }
            return format!("{mant}+{exp}");
        }
        return s;
    }
    if n.fract() == 0.0 {
        return format!("{n:.0}");
    }
    format!("{n}")
}

/// `ToInt32` — truncate then reduce modulo 2³².
pub fn to_int32(n: f64) -> i32 {
    if !n.is_finite() || n == 0.0 {
        return 0;
    }
    let m = n.trunc() % 4_294_967_296.0;
    let m = if m < 0.0 { m + 4_294_967_296.0 } else { m };
    // m ∈ [0, 2³²)
    if m >= 2_147_483_648.0 {
        (m - 4_294_967_296.0) as i64 as i32
    } else {
        m as i64 as i32
    }
}

/// `ToUint32`.
pub fn to_uint32(n: f64) -> u32 {
    to_int32(n) as u32
}

/// Whitespace JS au niveau **code unit** — `White_Space`
/// Unicode + `U+FEFF` (ZWNBSP, Cf hors `is_whitespace` Rust).
/// Un surrogate isolé n'est pas whitespace (spec `TrimString`).
fn unit_is_ws(u: u16) -> bool {
    char::from_u32(u32::from(u)).is_some_and(|c| c.is_whitespace() || c == '\u{feff}')
}
