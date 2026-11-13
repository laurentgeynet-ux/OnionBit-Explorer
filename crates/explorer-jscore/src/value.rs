// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Core JS value representation and number formatting.

use std::cell::Cell;
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
    /// Contenu UTF-8.
    s: String,
    /// Compteur d'octets vivants partagé avec l'`Engine`
    /// (`str_acct`) — incrémenté à l'alloc (avant
    /// l'emboîtement, pour que toute capacité comptée soit
    /// rendue), décrémenté à la mort du dernier handle.
    acct: Rc<Cell<isize>>,
}

impl Drop for StrBox {
    fn drop(&mut self) {
        self.acct.set(self.acct.get() - self.s.capacity() as isize);
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
            acct: acct.clone(),
        }))
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
        let old = b.s.capacity();
        b.s.push_str(rhs);
        b.acct.set(b.acct.get() + (b.s.capacity() - old) as isize);
        true
    }

    /// Longueur en **code units UTF-16** — le `length` spec
    /// d'une Str (astral = 2) sans la copie qu'un clone
    /// imposerait (§B-59).
    #[must_use]
    pub fn u16_len(&self) -> usize {
        self.as_str().encode_utf16().count()
    }

    /// Code unit UTF-16 à l'indice `i` (`charCodeAt`/`at` en
    /// boucle sans clone de la chaîne entière, §B-59).
    #[must_use]
    pub fn u16_at(&self, i: usize) -> Option<u16> {
        self.as_str().encode_utf16().nth(i)
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
    /// `slice`/`substring`/`substr` sans clone plein (§B-59).
    /// Surrogate isolé → U+FFFD (déviation documentée, régression
    /// JS-C263, verrou `c23_utf16_positions_and_slices`).
    #[must_use]
    pub fn slice_u16(&self, a: usize, b: usize) -> String {
        let units: Vec<u16> = self
            .as_str()
            .encode_utf16()
            .skip(a)
            .take(b.saturating_sub(a))
            .collect();
        String::from_utf16_lossy(&units)
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
        self.as_str() == other.as_str()
    }
}
impl Eq for JsStr {}

impl PartialEq<str> for JsStr {
    fn eq(&self, other: &str) -> bool {
        self.as_str() == other
    }
}
impl PartialEq<&str> for JsStr {
    fn eq(&self, other: &&str) -> bool {
        self.as_str() == *other
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
