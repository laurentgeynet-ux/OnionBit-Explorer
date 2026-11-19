// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! `RegExp` semantics for `explorer-jscore`, built on the pure-Rust
//! `fancy-regex` backtracker.
//!
//! Coverage: literals, `g i m s u y` flags, `exec`/`test` with
//! `lastIndex`, `String.match`/`replace`/`split`/`search`.
//!
//! Engine: `fancy-regex` (hybrid backtracker, pure Rust) —
//! look-around and backreferences (`(?=)`, `(?!`, `(?<=`, `(?<!`,
//! `\1`, `\k<name>`) compile natively. A `BACKTRACK_LIMIT` bound
//! caps hostile exponential patterns; an exhausted budget degrades
//! to "no match" rather than hanging (documented deviation — real
//! engines run on a watchdog, not a static bound).

/// A compiled JS regular expression plus its per-object flags.
#[allow(clippy::struct_excessive_bools)] // flags are flags.
pub struct JsRegExp {
    /// Compiled pattern (JS `i`/`m`/`s` folded into inline flags).
    re: fancy_regex::Regex,
    /// Pattern source as written (for `.source`/`toString`).
    pub source: String,
    /// `g` — global matching; drives `lastIndex` on exec/test.
    pub global: bool,
    /// `i` — case-insensitive.
    pub ignore_case: bool,
    /// `m` — `^`/`$` match at line boundaries.
    pub multiline: bool,
    /// `s` — `.` matches `\n`.
    pub dot_all: bool,
    /// `y` — match must start exactly at `lastIndex`.
    pub sticky: bool,
    /// `u` — accepted, always on (regex crate is Unicode-native).
    pub unicode: bool,
    /// `v` — unicodeSets accepted (`unicodeSets` getter; matching
    /// encore traité comme `u`).
    pub unicode_sets: bool,
    /// `d` — `exec` results carry `indices` (start/end per group).
    pub has_indices: bool,
    /// Le pattern compilé contient des chars **sentinelle**
    /// (`U+F0000..U+F07FF` = surrogate isolé) — le haystack doit
    /// alors passer par `enc_hay` pour un match unit-exact.
    pub has_sentinels: bool,
    /// Noms de groupes **synthétiques** `__eN` émis pour le
    /// backend (anonymes renommés quand `\N` cohabite avec des
    /// nommés — JS-C269). Filtrés des résultats `exec` pour ne
    /// pas fuiter dans `match.groups`/`indices.groups`.
    synth: std::collections::BTreeSet<String>,
    /// JS-C624 : noms de groupes renommés `synth → orig` —
    /// fancy-regex n'accepte que `[A-Za-z0-9_]` pour `?P<…>` ;
    /// les `IdentifierName` non-ASCII (`(?<$𐒤>)`) sont émis sous
    /// `__uN` et traduits à l'affichage (`groups`/`named`/`k`).
    renamed: std::collections::BTreeMap<String, String>,
}

/// **Domaine sentinelle regex** — même schéma que le lexer : un
/// surrogate isolé (UTF-16 `D800..DFFF` sans paire) devient le
/// char `U+F0000 + (u - 0xD800)` ; un vrai char de la zone
/// `U+F0000..=U+F0800` est préfixé d'un `U+F0800` échappeur.
/// `fancy-regex` ne peut pas voir de surrogate (UTF-8 valide
/// seulement) : pattern **et** haystack sont encodés dans ce
/// domaine quand l'un des deux en porte, ce qui rend le match
/// exact par code unit (`/\uFFFD/` ne matche plus `\uD800`).
pub(crate) fn re_encode(units: &[u16]) -> String {
    let mut out = String::with_capacity(units.len());
    for d in char::decode_utf16(units.iter().copied()) {
        match d {
            Ok(c) => {
                if (0xF_0000..=0xF_0800).contains(&(c as u32)) {
                    out.push('\u{F0800}');
                }
                out.push(c);
            }
            Err(e) => {
                let v = 0xF_0000 + u32::from(e.unpaired_surrogate()) - 0xD800;
                out.push(char::from_u32(v).unwrap_or('\u{FFFD}'));
            }
        }
    }
    out
}

/// **Domaine units éclatés** (regex non-`u`, JS-C622) : chaque
/// code unit → 1 char — tout surrogate (pairé **ou** isolé)
/// devient la sentinelle `U+F0000 + (u - 0xD800)`. Contrairement
/// à [`re_encode`], les paires NE sont PAS jointes : positions =
/// units 1:1 (`/\udca4/` matche dans `'𐒤'`, `.` = 1 unit,
/// `lastIndex` identité — sémantique code-unit du mode non-`u`).
pub(crate) fn re_encode_split(units: &[u16]) -> String {
    let mut out = String::with_capacity(units.len());
    for &u in units {
        let v = if (0xD800..0xE000).contains(&u) {
            0xF_0000 + u as u32 - 0xD800
        } else {
            u as u32
        };
        out.push(char::from_u32(v).unwrap_or('\u{FFFD}'));
    }
    out
}

/// Inverse de `re_encode` — reconstitue les code units UTF-16
/// exacts d'un texte du domaine sentinelle.
pub(crate) fn re_decode(s: &str) -> Vec<u16> {
    let mut units = Vec::with_capacity(s.len());
    let mut buf = [0u16; 2];
    let mut esc = false;
    for c in s.chars() {
        let v = c as u32;
        if esc {
            units.extend_from_slice(c.encode_utf16(&mut buf));
            esc = false;
        } else if v == 0xF_0800 {
            esc = true;
        } else if (0xF_0000..0xF_0800).contains(&v) {
            units.push((v - 0xF_0000 + 0xD800) as u16);
        } else {
            units.extend_from_slice(c.encode_utf16(&mut buf));
        }
    }
    units
}

/// Fragment classe **sentinelle** pour le code unit `cp`
/// (surrogate `D800..DFFF`) : `\x{f00xx}` — matche le surrogate
/// isolé exact dans le haystack encodé.
fn sent_frag(cp: u32) -> String {
    format!("\\x{{{:x}}}", 0xF_0000 + cp - 0xD800)
}

/// Fragment de plage astrale couverte par les surrogates
/// **leads** `D800..DBFF` de `[a_lo, a_hi]` (sémantique units
/// non-`u` : un lead de la plage matche l'astral à sa position).
/// `None` quand l'intersection est vide.
fn lead_astral_frag(a_lo: u32, a_hi: u32) -> Option<String> {
    let lo = a_lo.max(0xD800);
    let hi = a_hi.min(0xDBFF);
    if lo > hi {
        return None;
    }
    let base = 0x1_0000 + ((lo - 0xD800) << 10);
    let top = 0x1_0000 + ((hi - 0xD800) << 10) + 0x3FF;
    Some(format!("\\x{{{base:x}}}-\\x{{{top:x}}}"))
}

/// Carte char↔code-units UTF-16 pour les positions spec
/// (JS-C622) : `index`/`lastIndex`/`position`/`indices` sont en
/// **code units** côté JS alors que le moteur matche en chars du
/// domaine `hay` — astral = 1 char = 2 units, sentinelle = 1
/// unit, escaper `U+F0800` = 0 unit. `of` rend `None` quand le
/// haystack est pure BMP (char == unit — fast-path).
pub(crate) struct UnitMap {
    /// `u2c[u]` = index char où démarre le code unit `u` ; un
    /// unit médian de paire pointe la frontière char **suivante**
    /// (on ne peut pas commencer un scan au milieu d'un char).
    u2c: Vec<u32>,
    /// `c2u[ci]` = code unit où démarre le char `ci` — `len` =
    /// nb chars + 1 : la borne finale (fin de chaîne) est incluse.
    c2u: Vec<u32>,
}

impl UnitMap {
    /// Construit la carte pour `hay` (domaine char passé à
    /// `exec` — encodé ou brut) ; `None` si aucun char
    /// `≥ U+10000` (ni astral, ni sentinelle, ni escaper).
    pub fn of(hay: &str) -> Option<UnitMap> {
        if !hay.chars().any(|c| (c as u32) >= 0x1_0000) {
            return None;
        }
        let mut u2c: Vec<u32> = Vec::new();
        let mut c2u: Vec<u32> = Vec::new();
        let mut u = 0usize;
        let mut pending_esc: Option<usize> = None;
        for (ci, c) in hay.chars().enumerate() {
            let v = c as u32;
            if v == 0xF_0800 {
                // Escaper de `re_encode` — 0 unit propre ; ses
                // units appartiennent au char suivant et l'atome
                // est indivisible → `u2c` pointe l'escaper.
                c2u.push(u as u32);
                pending_esc = Some(ci);
                continue;
            }
            let w = if (0xF_0000..0xF_0800).contains(&v) {
                1 // sentinelle = surrogate isolé (1 unit)
            } else {
                c.len_utf16()
            };
            let start_ci = pending_esc.take().unwrap_or(ci);
            c2u.push(u as u32);
            u2c.resize(u + w, start_ci as u32);
            for k in 1..w {
                u2c[u + k] = (ci + 1) as u32;
            }
            u += w;
        }
        u2c.push(hay.chars().count() as u32);
        c2u.push(u as u32);
        Some(UnitMap { u2c, c2u })
    }

    /// Code unit → index char (clampé en borne haute).
    pub fn u2c(&self, u: usize) -> usize {
        self.u2c
            .get(u)
            .copied()
            .unwrap_or_else(|| *self.u2c.last().unwrap_or(&0)) as usize
    }

    /// Index char → code unit (borne finale comprise).
    pub fn c2u(&self, c: usize) -> usize {
        self.c2u
            .get(c)
            .copied()
            .unwrap_or(*self.c2u.last().unwrap_or(&0)) as usize
    }
}

/// One `exec` result, in *char* units (the engine's strings are
/// char-indexed — JS UTF-16 code units are approximated by chars).
pub struct ExecMatch {
    /// Char index of the whole match.
    pub index: usize,
    /// Char offset just past the whole match (new `lastIndex`).
    pub end: usize,
    /// `groups[0]` is the whole match; `groups[i]` is capture `i`.
    pub groups: Vec<Option<String>>,
    /// `(?<name>…)` captures — `None` quand le groupe déclaré
    /// n'a pas participé au match (spec : `groups` l'expose
    /// alors avec la valeur `undefined`).
    pub named: Vec<(String, Option<String>)>,
    /// Char-unit `[start, end)` per group (for `d`-flag `indices`).
    pub group_indices: Vec<Option<(usize, usize)>>,
    /// Char-unit `(start, end)` per named capture — `None` when
    /// the group did not participate (→ `undefined` in `indices.groups`).
    pub named_indices: Vec<(String, Option<(usize, usize)>)>,
}

impl JsRegExp {
    /// Parse flags and compile a JS pattern. `Err` carries a
    /// script-facing message (the caller wraps it in `SyntaxError`).
    pub fn compile(source: &str, flags: &str) -> Result<Self, String> {
        let mut global = false;
        let mut ignore_case = false;
        let mut multiline = false;
        let mut dot_all = false;
        let mut sticky = false;
        let mut unicode = false;
        let mut unicode_sets = false;
        let mut has_indices = false;
        for f in flags.chars() {
            match f {
                'g' if !global => global = true,
                'i' if !ignore_case => ignore_case = true,
                'm' if !multiline => multiline = true,
                's' if !dot_all => dot_all = true,
                'y' if !sticky => sticky = true,
                'u' if !unicode => unicode = true,
                'd' if !has_indices => has_indices = true,
                'v' if !unicode_sets => unicode_sets = true,
                _ => return Err(format!("invalid regex flag `{f}`")),
            }
        }
        // Spec : `u` et `v` sont mutuellement exclusifs (`/x/uv`
        // = SyntaxError).
        if unicode && unicode_sets {
            return Err("invalid regex flags `uv`".to_string());
        }
        let mut pat = String::new();
        // `i` : le backend fait du folding Unicode complet — exact
        // en mode `u`, trop large en non-`u` où JS canonise par
        // `ToUppercase` simple (`/\u{212a}/i` ne doit pas matcher
        // 'k'). Non-`u` → folding manuel dans `translate_pattern`.
        if ignore_case && (unicode || unicode_sets) {
            pat.push_str("(?i)");
        }
        if multiline {
            pat.push_str("(?m)");
        }
        if dot_all {
            pat.push_str("(?s)");
        }
        // `v` ≈ `u` côté moteur (unicodeSets non implémenté) —
        // même traduction unicode.
        let uni = unicode || unicode_sets;
        let (translated, synth, renamed) =
            translate_pattern(source, uni, unicode_sets, dot_all, ignore_case && !uni)?;
        pat.push_str(&translated);
        // Domaine sentinelle : le pattern traduit porte des
        // chars `F0000..=F0800` (surrogates du source ou d'un
        // JsStr à units via `re_encode`) → haystack encodé.
        let has_sentinels = pat
            .chars()
            .any(|c| (0xF_0000..=0xF_0800).contains(&(c as u32)));
        let re = fancy_regex::RegexBuilder::new(&pat)
            .backtrack_limit(BACKTRACK_LIMIT)
            .delegate_size_limit(DELEGATE_SIZE_LIMIT)
            .build()
            .map_err(|e| format!("invalid regex /{source}/: {}", short_regex_error(&e)))?;
        Ok(JsRegExp {
            re,
            source: source.to_string(),
            global,
            ignore_case,
            multiline,
            dot_all,
            sticky,
            unicode,
            unicode_sets,
            has_indices,
            has_sentinels,
            synth,
            renamed,
        })
    }

    /// `/src/flags` rendering (`toString`) — ordre canonique
    /// `dgimsuy` (spec `RegExp.prototype.flags`).
    /// `EscapeRegExpPattern` spec : le `.source` exposé échappe
    /// `/` non déjà échappé → `\/` et les fins de ligne — le
    /// pattern reparse à l'identique dans un littéral `//`.
    pub fn escaped_source(&self) -> String {
        escape_regexp_source(&self.source)
    }

    pub fn flag_string(&self) -> String {
        let mut f = String::new();
        if self.has_indices {
            f.push('d');
        }
        if self.global {
            f.push('g');
        }
        if self.ignore_case {
            f.push('i');
        }
        if self.multiline {
            f.push('m');
        }
        if self.dot_all {
            f.push('s');
        }
        if self.unicode {
            f.push('u');
        }
        if self.unicode_sets {
            f.push('v');
        }
        if self.sticky {
            f.push('y');
        }
        f
    }

    /// Haystack prêt pour le moteur : **encodé** dans le domaine
    /// sentinelle (`re_encode`) quand le pattern en porte ou que
    /// le sujet a des `units` — sinon passthrough. Quand le
    /// second retour est `true`, tout texte issu du match (match,
    /// groupes, pièces de split, contexte de splice) est dans le
    /// domaine encodé et se décode via `re_decode`.
    pub(crate) fn enc_hay<'h>(
        &self,
        hay: &'h crate::value::JsStr,
    ) -> (std::borrow::Cow<'h, str>, bool) {
        if !(self.unicode || self.unicode_sets) {
            // Non-`u` : domaine **units éclatés** (JS-C622) —
            // positions = units 1:1, mid-paire matchable. Ne
            // s'applique que si le sujet porte des surrogates
            // (astral ou isolé) ; sinon passthrough.
            if !hay.has_units() && !hay.as_str().chars().any(|c| (c as u32) >= 0x1_0000) {
                return (std::borrow::Cow::Borrowed(hay.as_str()), false);
            }
            return (
                std::borrow::Cow::Owned(re_encode_split(&hay.units_view())),
                true,
            );
        }
        if self.has_sentinels || hay.has_units() {
            (std::borrow::Cow::Owned(re_encode(&hay.units_view())), true)
        } else {
            (std::borrow::Cow::Borrowed(hay.as_str()), false)
        }
    }

    /// First match at or after `start` (char units). With `sticky`,
    /// the match must begin exactly at `start`.
    pub fn exec(&self, hay: &str, start: usize) -> Option<ExecMatch> {
        crate::j17_tick(&crate::J17_REGEX);
        // Spec `RegExpBuiltinExec` : `lastIndex > length` → pas
        // de match (le clamp de `char_to_byte` ferait sinon
        // matcher un motif vide à la fin, boucle infinie).
        if start > hay.chars().count() {
            return None;
        }
        let byte_start = char_to_byte(hay, start);
        let caps = self.re.captures_from_pos(hay, byte_start).ok()??;
        let m = caps.get(0)?;
        if self.sticky && byte_to_char(hay, m.start()) != start {
            return None;
        }
        Some(self.build_match(hay, &caps))
    }

    /// `exec` driving `lastIndex`: `g`/`y` read it and spec
    /// `RegExpBuiltinExec` writes back **endIndex** — jamais
    /// bumpé sur match vide (l'avance est à la charge des
    /// boucles appelantes : `exec_all`, `@@match`, `@@split`).
    pub fn exec_from(&self, hay: &str, last_index: usize) -> Option<(ExecMatch, usize)> {
        let m = self.exec(hay, last_index)?;
        let end = m.end;
        Some((m, end))
    }

    fn build_match(&self, hay: &str, caps: &fancy_regex::Captures) -> ExecMatch {
        let m = caps.get(0);
        let (index, end, text) = match m {
            Some(m) => (
                byte_to_char(hay, m.start()),
                byte_to_char(hay, m.end()),
                m.as_str().to_string(),
            ),
            None => (0, 0, String::new()),
        };
        let mut groups = Vec::with_capacity(caps.len());
        groups.push(Some(text.clone()));
        for i in 1..caps.len() {
            groups.push(caps.get(i).map(|g| g.as_str().to_string()));
        }
        let mut named = Vec::new();
        let mut named_indices = Vec::new();
        for name in self
            .re
            .capture_names()
            .flatten()
            .filter(|n| !self.synth.contains(*n))
        {
            // JS-C624 : le nom émis (`__uN`) redevient l'origine
            // (`$𐒤`) à l'affichage.
            let disp = self
                .renamed
                .get(name)
                .map_or_else(|| name.to_string(), Clone::clone);
            named.push((
                disp.clone(),
                caps.name(name).map(|g| g.as_str().to_string()),
            ));
            named_indices.push((
                disp,
                caps.name(name)
                    .map(|g| (byte_to_char(hay, g.start()), byte_to_char(hay, g.end()))),
            ));
        }
        let mut group_indices = Vec::with_capacity(caps.len());
        for i in 0..caps.len() {
            group_indices.push(
                caps.get(i)
                    .map(|g| (byte_to_char(hay, g.start()), byte_to_char(hay, g.end()))),
            );
        }
        ExecMatch {
            index,
            end,
            groups,
            named,
            group_indices,
            named_indices,
        }
    }
}

/// `(?<name>…)` → `(?P<name>…)`; everything else passes through
/// `EscapeRegExpPattern` — échappe `/` **non déjà échappé**
/// (délimiteur de littéral) et les fins de ligne (`\n`, `\r`,
/// `U+2028`, `U+2029`) ; tout le reste passe verbatim. La parité
/// de la série de `\` précédents tranche : `/` après un nombre
/// **impair** de `\` fait déjà partie d'une paire `\/` (reparse
/// inchangé) ; après une série **paire** (dont zéro) il ferme
/// le littéral → `\/`. Échapper un `\/` existant produisait
/// `a\\/b` — non reparseable, la seule exigence normative de
/// l'opération (mesuré : jscore ET boa jetaient `SyntaxError` sur
/// `eval('/'+/a\/b/.source+'/')`, V8 repasse — JS-C586).
pub(crate) fn escape_regexp_source(src: &str) -> String {
    let mut out = String::with_capacity(src.len() + 8);
    let mut backslashes = 0u32;
    for c in src.chars() {
        match c {
            '/' if backslashes % 2 == 0 => out.push_str("\\/"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\u{2028}' => out.push_str("\\u2028"),
            '\u{2029}' => out.push_str("\\u2029"),
            _ => out.push(c),
        }
        backslashes = if c == '\\' { backslashes + 1 } else { 0 };
    }
    out
}

/// Sortie de [`translate_pattern`] : pattern traduit, noms
/// synthétiques `__eN` (anonymes), renommages `synth → orig`
/// des `IdentifierName` non-ASCII (JS-C624).
type TranslateOut = (
    String,
    std::collections::BTreeSet<String>,
    std::collections::BTreeMap<String, String>,
);

/// JS source → `fancy-regex` pattern, with JS-side validation the
/// backtracker doesn't do (it is *more* permissive than ECMA-262):
///
/// - `(?<name>…)` → `(?P<name>…)`, names validated as
///   `IdentifierName` and checked for duplicates;
/// - inline modifiers `(?ims:…)`/`(?ims-ims:…)` validated per
///   ES2025 (`ims` only, no dup, disjoint sides, `:` mandatory) —
///   `(?u:…)`, duplicates or a missing colon are `SyntaxError`;
/// - `{n}`/`{n,}`/`{n,m}` quantifiers need a preceding atom and
///   `n ≤ m`; bare `{`/`}`/`]` are `SyntaxError` in `u` mode
///   (extended pattern chars), literals in sloppy Annex B;
/// - `\u{hex}` normalised to `\x{hex}` (leading zeros stripped,
///   `≤ 0x10FFFF`);
/// - class ranges: `lo > hi` → `SyntaxError` (both modes); in `u`
///   a bound may not be a class escape (`[\d-z]`/`[a-\d]`/`[--\d]`
///   → `SyntaxError`) — sloppy keeps the `-` literal (`\-`);
/// - `(?P…`/`(?#` rejected (non-JS forms the crate accepts);
/// - `\0` → `\x{0}`, `\cX` → control char, surrogate pairs fused
///   in `u` mode, identity escapes Annex B sloppy (strict in `u`);
/// - `\k<name>`/`\1`–`\9` backrefs pass through natively —
///   `\k<name>` before its group declaration is rewritten to an
///   empty match (spec 21.2.2.9); out-of-range `\N` falls back to
///   Annex B (valid prefix, legacy octal, `\8`/`\9` identity);
/// - `fold` (`i` without `u`): case folding emitted by hand —
///   `[cCU]` classes from `ToUppercase`/`ToLowercase` simple
///   (canonicalisation guard: `upper(v) == upper(c)`), because the
///   backend's `i` is full Unicode folding — `/\u{212a}/i` must
///   not match 'k' (KELVIN has no JS-canonical case partner).
///
/// Renvoie `(pattern traduit, noms synthétiques `__eN` anonymes,
/// renommages `synth → orig` des noms non-ASCII JS-C624)`.
#[allow(clippy::fn_params_excessive_bools)]
fn translate_pattern(
    src: &str,
    unicode: bool,
    vsets: bool,
    dotall: bool,
    fold: bool,
) -> Result<TranslateOut, String> {
    let cs: Vec<char> = src.chars().collect();
    let (all_names, n_groups, has_decimal) = prescan(&cs);
    // JS-C269 : `fancy_regex` refuse `\N` dès qu'un groupe nommé
    // existe (« Numbered backref not allowed ») — restriction du
    // backend, pas du JS. Quand les deux cohabitent, les groupes
    // anonymes sont renommés `(?P<__eN>` et `\N` réémis `\k<…>` ;
    // les noms synthétiques sont filtrés à l'exec pour ne pas
    // polluer `match.groups`/`indices.groups`.
    let need_synth = !all_names.is_empty() && has_decimal;
    let mut t = Trans {
        unicode,
        vsets,
        dotall,
        fold,
        n_groups,
        names: std::collections::BTreeSet::new(),
        all_names,
        emitted: Vec::new(),
        need_synth,
        synth: std::collections::BTreeSet::new(),
        renamed: std::collections::BTreeMap::new(),
        orig_to_synth: std::collections::BTreeMap::new(),
        groups_open: 0,
        open_stack: Vec::new(),
        look_pos: Vec::new(),
        closed: std::collections::BTreeSet::new(),
    };
    let mut out = String::with_capacity(src.len() + 8);
    let mut i = 0;
    let mut in_class = false;
    // Un atome précède-t-il la position courante — requis pour un
    // quantificateur `{n}` (`/{2}/` est toujours une erreur).
    let mut last_atom = false;
    // Atome de classe précédent — validation des ranges.
    let mut class_prev: Option<ClassAtom> = None;
    // `-` de range émis : le prochain atome en est la borne haute.
    let mut class_dash = false;
    // L'atome précédent était une borne de range — un `-` qui suit
    // est littéral, jamais le début d'une autre range (`[a-b-c]`).
    let mut class_prev_bound = false;
    // Suivi d'émission de la classe courante : les atomes
    // surrogate (jamais dans nos strings UTF-8) sont omis — une
    // classe entièrement vidée dégénère en `(?!)` / `[\s\S]`.
    let mut class_start_out = 0usize;
    let mut class_negated = false;
    let mut class_emitted = false;
    while i < cs.len() {
        let c = cs[i];
        if c == '\\' {
            // `\p`/`\P` sloppy **en classe** : identité
            // littérale `p`/`P` (Annex B — Chrome matche la
            // lettre, pas une propriété : `[^\p{Pat_Syn}
            // \p{Pat_WS}]` d'airfrance est le set négatif
            // `{p,{,P,a,t,…}}`). `escape` rendait `\p` nu
            // et le `{Nom}` suivant devenait des littéraux
            // échappés → le backend lisait `\p\x{7b}…` →
            // « error parsing pattern » (§B-56 bras 2,
            // vol 98). En `u`, `\p{…}` reste une vraie
            // propriété — valide en classe côté backend.
            if in_class && !t.unicode && matches!(cs.get(i + 1), Some('p' | 'P')) {
                let ch = u32::from(cs[i + 1]);
                class_char(&mut out, ch);
                class_emitted = true;
                if t.fold {
                    let is_bound = class_dash
                        || (cs.get(i + 2) == Some(&'-')
                            && !matches!(class_atom_of(&cs, i + 3), ClassAtom::End));
                    if !is_bound {
                        emit_fold_variants(&mut out, ch);
                    }
                }
                class_prev = Some(ClassAtom::Char(ch));
                class_prev_bound = class_dash;
                class_dash = false;
                i += 2;
                continue;
            }
            // `\N` chiffre **en classe** : toujours octale /
            // identité Annex B, jamais une backref — `escape`
            // rendait `\1(?:00)` pour `\100` → backref dans la
            // classe → « Invalid character class » (§B-56
            // bras 3, jscrambler airfrance vol 101). En `u`
            // l'échappe reste une erreur — la voie `escape`
            // ci-dessous rejette proprement.
            if in_class && !t.unicode && matches!(cs.get(i + 1), Some('0'..='9')) {
                let (v, next) = class_octal(&cs, i);
                // `\x{…}` comme `escape()` — sûr pour les
                // spéciaux de classe (`[`/`&`/`~`/`-`) et
                // uniforme avec le format octal d'avant.
                out.push_str(&format!("\\x{{{v:x}}}"));
                class_emitted = true;
                if t.fold {
                    let is_bound = class_dash
                        || (cs.get(next) == Some(&'-')
                            && !matches!(class_atom_of(&cs, next + 1), ClassAtom::End));
                    if !is_bound {
                        emit_fold_variants(&mut out, v);
                    }
                }
                class_prev = Some(ClassAtom::Char(v));
                class_prev_bound = class_dash;
                class_dash = false;
                i = next;
                continue;
            }
            let (frag, next) = t.escape(&cs, i, src)?;
            // JS-C625 : `\s` JS = `White_Space` Unicode **+ FEFF**
            // (BOM/ZWNBSP) — le `\s` backend est le set Unicode
            // sans FEFF. Hors classe : atome classe élargie ;
            // en classe : membre `\x{feff}` supplémentaire
            // (couvre `[^\s]` = `\S` correctement). `\S` nu en
            // classe reste la limite connue (union non-négative).
            let frag = match frag.as_str() {
                "\\s" if in_class => "\\s\\x{feff}".to_string(),
                "\\s" => "[\\s\\x{feff}]".to_string(),
                "\\S" if !in_class => "[^\\s\\x{feff}]".to_string(),
                _ => frag,
            };
            if in_class {
                let atom = class_atom_of(&cs, i);
                if let ClassAtom::Char(cp) = atom {
                    if is_surrogate(cp) {
                        // Suivi de `-` → borne de range : rien
                        // n'est émis ici, l'union sentinelle /
                        // scalaire / astrale sort au site range.
                        // Sinon, atome = sentinelle du code unit
                        // (+ astrals du lead, sémantique units
                        // non-`u`).
                        let is_bound = cs.get(next) == Some(&'-')
                            && !matches!(class_atom_of(&cs, next + 1), ClassAtom::End);
                        if !is_bound {
                            out.push_str(&sent_frag(cp));
                            if !unicode {
                                if let Some(f) = lead_astral_frag(cp, cp) {
                                    out.push_str(&f);
                                }
                            }
                            class_emitted = true;
                        }
                        class_prev = Some(atom);
                        class_prev_bound = class_dash;
                        class_dash = false;
                        i = next;
                        continue;
                    }
                    // Paire surrogates fusionnée par `class_atom_of`
                    // (`\ud83d\ude00` → astral) : `frag` porte le
                    // premier échappe brut (`\x{d83d}` — codepoint
                    // invalide pour le backend, hors `u` la fusion
                    // n'est pas faite côté `escape`) → émettre le
                    // point astral, sauter les 12 chars de la paire.
                    if cp > 0xFFFF {
                        if let Some((_cp, pnext)) = surrogate_pair(&cs, i) {
                            if unicode {
                                out.push_str(&format!("\\x{{{cp:x}}}"));
                                class_prev = Some(atom);
                            } else {
                                // JS-C622 : atome astral non-`u` =
                                // 2 membres sentinels (units).
                                let a = cp - 0x1_0000;
                                out.push_str(&sent_frag(0xD800 + (a >> 10)));
                                out.push_str(&sent_frag(0xDC00 + (a & 0x3FF)));
                                class_prev = Some(ClassAtom::Char(0xF_0400 + (a & 0x3FF)));
                            }
                            class_emitted = true;
                            class_prev_bound = class_dash;
                            class_dash = false;
                            i = pnext;
                            continue;
                        }
                    }
                }
                out.push_str(&frag);
                class_emitted = true;
                // Fold individuel — jamais sur une borne de range
                // (le `-` émet le fold de la range en un bloc).
                if t.fold {
                    let is_bound = class_dash
                        || (cs.get(next) == Some(&'-')
                            && !matches!(class_atom_of(&cs, next + 1), ClassAtom::End));
                    if !is_bound {
                        if let ClassAtom::Char(cp) = atom {
                            emit_fold_variants(&mut out, cp);
                        }
                    }
                }
                class_prev = Some(atom);
                class_prev_bound = class_dash;
                class_dash = false;
            } else {
                if let ClassAtom::Char(cp) = class_atom_of(&cs, i) {
                    if is_surrogate(cp) {
                        // `\uD8hi[\uDClo-\uDChi]` — idiom emoji des
                        // bundles (`\uD83C[\uDFFB-\uDFFF]` = skin
                        // tone) : surrogate haute + classe de
                        // basses = plage astrale.
                        if (0xd800..0xdc00).contains(&cp) {
                            if let Some((frag, anext)) = astral_class(&cs, next, cp, unicode) {
                                out.push_str(&frag);
                                last_atom = true;
                                i = anext;
                                continue;
                            }
                        }
                        // Domaine sentinelle : `\x{f00xx}` matche
                        // le code unit isolé exact ; non-`u`, un
                        // lead couvre aussi l'astral qu'il préfixe
                        // (sa moitié haute est présente à sa
                        // position — sémantique units).
                        if !unicode && (0xd800..0xdc00).contains(&cp) {
                            if let Some(af) = lead_astral_frag(cp, cp) {
                                out.push_str(&format!("[{}{}]", sent_frag(cp), af));
                                last_atom = true;
                                i = next;
                                continue;
                            }
                        }
                        out.push_str(&sent_frag(cp));
                        last_atom = true;
                        i = next;
                        continue;
                    }
                    // Paire de surrogates fusionnée par
                    // `class_atom_of` (`\ud83d\udca5`, pattern emoji
                    // des bundles minifiés — GitLab) : `frag` ne
                    // porte que le premier échappe brut
                    // (`\x{d83d}`, invalide pour le backend hors
                    // mode `u` où `escape` ne fusionne pas) →
                    // émettre le point astral et sauter les 12
                    // chars de la paire.
                    if cp > 0xFFFF {
                        if let Some((_cp, pnext)) = surrogate_pair(&cs, i) {
                            if unicode {
                                out.push_str(&format!("\\x{{{cp:x}}}"));
                            } else {
                                // JS-C622 : domaine units éclatés —
                                // la paire fusionnée vaut ses 2
                                // sentinelles, groupées en un atome
                                // (`/x*/` quantifie la paire).
                                let a = cp - 0x1_0000;
                                out.push_str("(?:");
                                out.push_str(&sent_frag(0xD800 + (a >> 10)));
                                out.push_str(&sent_frag(0xDC00 + (a & 0x3FF)));
                                out.push(')');
                            }
                            last_atom = true;
                            i = pnext;
                            continue;
                        }
                    }
                }
                if frag.starts_with("\\x{f0") && frag.contains("}\\x{f0") {
                    // JS-C622 : `\u{…}` astral non-`u` = paire de
                    // sentinelles — groupée en un atome pour les
                    // quantificateurs.
                    out.push_str("(?:");
                    out.push_str(&frag);
                    out.push(')');
                } else {
                    emit_escape(&mut out, &frag, t.fold);
                }
                last_atom = true;
            }
            i = next;
            continue;
        }
        match c {
            '[' if !in_class && t.vsets => {
                // JS-C625 : grammaire `v` — `v_class` parse la
                // classe entière (imbrication/ops/strings) et
                // rend un atome complet.
                let (frag, next) = v_class(&mut t, &cs, i, src)?;
                out.push_str(&frag);
                last_atom = true;
                i = next;
                continue;
            }
            '[' if !in_class => {
                in_class = true;
                class_prev = None;
                class_dash = false;
                class_prev_bound = false;
                class_start_out = out.len();
                class_emitted = false;
                out.push('[');
                // `[^` — la négation n'est pas un atome.
                class_negated = cs.get(i + 1) == Some(&'^');
                if class_negated {
                    out.push('^');
                    i += 1;
                }
            }
            ']' if in_class => {
                in_class = false;
                last_atom = true;
                if class_emitted {
                    out.push(']');
                } else {
                    // Classe vidée par les surrogates omis :
                    // `[^surr]` matche tout, `[surr]` jamais.
                    out.truncate(class_start_out);
                    out.push_str(if class_negated { "[\\s\\S]" } else { "(?!)" });
                }
            }
            ']' if unicode => {
                // `]` nu : littéral sloppy (Annex B), erreur en `u`.
                return Err(format!("unmatched `]` in `u` regex /{src}/"));
            }
            ']' => {
                last_atom = true;
                push_folded(&mut out, ']', t.fold);
            }
            '-' if in_class => {
                let next = class_atom_of(&cs, i + 1);
                let is_range =
                    class_prev.is_some() && !class_prev_bound && !matches!(next, ClassAtom::End);
                let escapes = matches!(class_prev, Some(ClassAtom::Escape))
                    || matches!(next, ClassAtom::Escape);
                if is_range && escapes {
                    if unicode {
                        return Err(format!("invalid class range in `u` regex /{src}/"));
                    }
                    // Sloppy : `-` littéral devant une class-escape
                    // (`\-` n'est pas un littéral en classe pour le
                    // backend — `\x{2d}` uniformément).
                    class_char(&mut out, u32::from('-'));
                    class_emitted = true;
                    class_prev = Some(ClassAtom::Char(u32::from('-')));
                    class_prev_bound = false;
                } else if is_range {
                    if let (Some(ClassAtom::Char(lo)), ClassAtom::Char(hi)) = (class_prev, next) {
                        if lo > hi {
                            return Err(format!("empty class range in /{src}/"));
                        }
                        // La borne haute est toujours consommée ici
                        // (fold ou non) — l'émission décalée au tour
                        // suivant empêcherait la sémantique units.
                        let (hfrag, hnext) = if cs[i + 1] == '\\' {
                            // Borne haute = paire surrogates
                            // fusionnée (`-\ud83d\ude4f`) : émettre
                            // le point astral, pas `\x{d83d}` brut.
                            if hi > 0xFFFF {
                                if let Some((_cp, pnext)) = surrogate_pair(&cs, i + 1) {
                                    (format!("\\x{{{hi:x}}}"), pnext)
                                } else {
                                    t.escape(&cs, i + 1, src)?
                                }
                            } else {
                                t.escape(&cs, i + 1, src)?
                            }
                        } else {
                            (cs[i + 1].to_string(), i + 2)
                        };
                        let lo_c = if is_surrogate(lo) { 0xe000 } else { lo };
                        let hi_c = if is_surrogate(hi) { 0xd7ff } else { hi };
                        if !unicode && hi > 0xFFFF && !is_surrogate(lo) {
                            // JS-C622 : borne haute astrale non-`u`
                            // = range units `lo..D7FF` + sentinelles
                            // `D800..lead` + atome trail (domaine
                            // units éclatés).
                            let a = hi - 0x1_0000;
                            out.push_str(&format!(
                                "-\\x{{d7ff}}\\x{{f0000}}-\\x{{{:x}}}\\x{{{:x}}}",
                                0xF_0000 + (a >> 10),
                                0xF_0400 + (a & 0x3FF)
                            ));
                            class_emitted = true;
                            class_prev = Some(ClassAtom::Char(0xF_0400 + (a & 0x3FF)));
                            class_prev_bound = true;
                            class_dash = false;
                            i = hnext;
                            continue;
                        }
                        if lo_c <= hi_c {
                            // La borne basse n'a été émise que si
                            // non-surrogate — sinon on l'émet ici
                            // clampée.
                            if is_surrogate(lo) {
                                class_char(&mut out, lo_c);
                            }
                            out.push('-');
                            if is_surrogate(hi) {
                                class_char(&mut out, hi_c);
                            } else if cs[i + 1] != '\\' {
                                // Borne haute brute — `class_char`
                                // échappe la syntaxe de classe
                                // backend (`[`/`&`/`~`) et uniformise
                                // la sortie (`\x{…}`).
                                class_char(&mut out, u32::from(cs[i + 1]));
                            } else {
                                out.push_str(&hfrag);
                            }
                            class_emitted = true;
                            if t.fold {
                                // La range est émise en un bloc —
                                // un fold individuel des bornes
                                // produirait des ranges invalides.
                                for map in [simple_upper, simple_lower] {
                                    if let (Some(flo), Some(fhi)) = (map(lo_c), map(hi_c)) {
                                        // La variante de la borne
                                        // doit se canoniser comme
                                        // la borne.
                                        if flo <= fhi
                                            && simple_upper(flo).unwrap_or(flo)
                                                == simple_upper(lo_c).unwrap_or(lo_c)
                                            && simple_upper(fhi).unwrap_or(fhi)
                                                == simple_upper(hi_c).unwrap_or(hi_c)
                                        {
                                            class_char(&mut out, flo);
                                            out.push('-');
                                            class_char(&mut out, fhi);
                                        }
                                    }
                                }
                            }
                        }
                        // Domaine sentinelle : les units surrogate
                        // de `[lo,hi]` matchent aussi les
                        // sentinelles encodées (surrogate isolé) —
                        // membre-range supplémentaire de la classe.
                        let surr_lo = lo.max(0xd800);
                        let surr_hi = hi.min(0xdfff);
                        if surr_lo <= surr_hi {
                            out.push_str(&format!(
                                "\\x{{{:x}}}-\\x{{{:x}}}",
                                0xF_0000 + surr_lo - 0xD800,
                                0xF_0000 + surr_hi - 0xD800
                            ));
                            class_emitted = true;
                        }
                        // Astrals dont le lead est couvert
                        // (units non-`u`) — en `u`, l'astral est
                        // atome indivisible hors plage surrogate.
                        if !unicode {
                            if let Some(af) = lead_astral_frag(lo, hi) {
                                out.push_str(&af);
                                class_emitted = true;
                            }
                        }
                        class_prev = Some(ClassAtom::Char(hi));
                        class_prev_bound = true;
                        class_dash = false;
                        i = hnext;
                        continue;
                    }
                    out.push('-');
                    class_dash = true;
                } else {
                    // `-` littéral (tête/queue de classe, ou après
                    // une borne de range) — `\x{2d}` : `\-` en
                    // classe n'est pas un littéral pour le backend
                    // (`/[-]/` compilait sans jamais matcher —
                    // classe vidée → `(?!)`, et `[-]?` rejeté —
                    // ecma-international). `class_emitted` requis :
                    // sans lui la classe dégénère en `(?!)`.
                    class_char(&mut out, u32::from('-'));
                    class_emitted = true;
                    class_prev = Some(ClassAtom::Char(u32::from('-')));
                    class_prev_bound = false;
                }
            }
            '(' if !in_class && cs.get(i + 1) == Some(&'?') => {
                i = translate_group(&mut t, &cs, i, &mut out, src)?;
                last_atom = false;
                continue;
            }
            '(' if !in_class => {
                t.groups_open += 1;
                t.open_stack.push(Some(t.groups_open));
                t.look_pos.push(None);
                last_atom = false;
                if t.need_synth {
                    // JS-C269 : nommer l'anonyme pour que `\N`
                    // puisse être réémis `\k<__eN>` (le backend
                    // refuse `\N` dès qu'un nommé existe).
                    let nm = t.synth_name(t.groups_open);
                    out.push_str("(?P<");
                    out.push_str(&nm);
                    out.push('>');
                } else {
                    t.emitted.push(String::new());
                    out.push('(');
                }
            }
            ')' if !in_class => {
                if let Some(Some(n)) = t.open_stack.pop() {
                    t.closed.insert(n);
                }
                last_atom = true;
                out.push(')');
                if let Some(pos) = t.look_pos.pop().flatten() {
                    // Lookahead quantifié (Annex B) : `(?=x)?` —
                    // le backend refuse le quantificateur sur
                    // assertion ; réécriture équivalente. Une
                    // assertion zéro-largeur est idempotente :
                    // min=0 → optionnelle, min≥1 → forme nue. Le
                    // quantificateur (et un `?` lazy) est absorbé.
                    let q = match cs.get(i + 1) {
                        Some('*' | '?') => Some((0usize, i + 2)),
                        Some('+') => Some((1, i + 2)),
                        Some('{') => {
                            parse_quantifier(&cs, i + 1).map(|(lo, _hi, next)| (lo as usize, next))
                        }
                        _ => None,
                    };
                    if let Some((lo, mut qend)) = q {
                        // Annex B — non-`u` uniquement ; `u` exige
                        // SyntaxError (test262 `u-invalid-*-lookahead`).
                        if t.unicode {
                            return Err(format!("quantifiable lookahead in `u` regex /{src}/"));
                        }
                        if cs.get(qend) == Some(&'?') {
                            qend += 1;
                        }
                        if lo == 0 {
                            let g = out[pos..].to_string();
                            out.truncate(pos);
                            out.push_str("(?:");
                            out.push_str(&g);
                            out.push_str("|)");
                        }
                        i = qend - 1;
                    }
                }
            }
            '{' | '}' if !in_class => {
                // `{n,m}` quantifier → atome précédent requis ; la
                // forme non-quantificateur est un littéral sloppy,
                // une erreur `u` (extended pattern chars — `}`
                // orphelin idem).
                if let Some((lo, hi, next)) = parse_quantifier(&cs, i) {
                    if !last_atom {
                        return Err(format!("quantifier without atom in /{src}/"));
                    }
                    if let Some(hi) = hi {
                        if lo > hi {
                            return Err(format!("invalid quantifier range in /{src}/"));
                        }
                    }
                    out.push_str(&cs[i..next].iter().collect::<String>());
                    last_atom = false;
                    i = next;
                    // `?` modifieur lazy (`{n,m}?`) — syntaxe
                    // identique côté backend, émis tel quel.
                    if cs.get(i) == Some(&'?') {
                        out.push('?');
                        i += 1;
                    }
                    continue;
                }
                if unicode {
                    return Err(format!("lone `{c}` in `u` regex /{src}/"));
                }
                last_atom = true;
                push_folded(&mut out, c, t.fold);
            }
            '|' if !in_class => {
                last_atom = false;
                out.push('|');
            }
            '^' | '$' if !in_class => {
                // Annex B sloppy : les assertions sont
                // « quantifiables » (`^*` toléré) — `u` exige un
                // vrai atome.
                last_atom = !unicode;
                out.push(c);
            }
            '*' | '+' | '?' if !in_class => {
                if !last_atom {
                    return Err(format!("quantifier without atom in /{src}/"));
                }
                last_atom = false;
                out.push(c);
                // `?` modifieur lazy (`*?`/`+?`/`??`) — syntaxe
                // identique côté backend, émis tel quel.
                if cs.get(i + 1) == Some(&'?') {
                    out.push('?');
                    i += 1;
                }
            }
            '.' if !in_class => {
                last_atom = true;
                // JS-C625 : `.` exclut les 4 `LineTerminator` —
                // le `.` fancy n'exclut que `\n`. Sous `s` :
                // tout char (`[\s\S]`).
                if t.dotall {
                    out.push_str("[\\s\\S]");
                } else {
                    out.push_str("[^\\n\\r\u{2028}\u{2029}]");
                }
            }
            _ => {
                if in_class && !unicode && (c as u32) >= 0x1_0000 {
                    // JS-C622 : atome astral non-`u` = 2 membres
                    // sentinels (lead + trail — units éclatés).
                    let a = c as u32 - 0x1_0000;
                    out.push_str(&sent_frag(0xD800 + (a >> 10)));
                    out.push_str(&sent_frag(0xDC00 + (a & 0x3FF)));
                    class_emitted = true;
                    class_prev = Some(ClassAtom::Char(0xF_0400 + (a & 0x3FF)));
                    class_prev_bound = class_dash;
                    class_dash = false;
                } else if in_class {
                    // Littéral de classe via `class_char` :
                    // `[`/`&`/`~` ont une syntaxe en classe côté
                    // backend (set/intersection/diff) — le brut
                    // `[.*+?^${}()|[\]\\]` du web réel rejetait
                    // « Invalid character class » (JS-C43). La
                    // sortie est uniformisée en `\x{…}`.
                    class_char(&mut out, u32::from(c));
                    class_emitted = true;
                    if t.fold {
                        let is_bound = class_dash
                            || (cs.get(i + 1) == Some(&'-')
                                && !matches!(class_atom_of(&cs, i + 2), ClassAtom::End));
                        if !is_bound {
                            emit_fold_variants(&mut out, u32::from(c));
                        }
                    }
                    class_prev = Some(ClassAtom::Char(u32::from(c)));
                    class_prev_bound = class_dash;
                    class_dash = false;
                } else if !unicode && (c as u32) >= 0x1_0000 {
                    // JS-C622 : atome astral non-`u` = ses 2 code
                    // units — paire de sentinelles groupée en un
                    // atome (domaine units éclatés ; `/x*/`
                    // quantifie la paire).
                    let cp = c as u32 - 0x1_0000;
                    out.push_str("(?:");
                    out.push_str(&sent_frag(0xD800 + (cp >> 10)));
                    out.push_str(&sent_frag(0xDC00 + (cp & 0x3FF)));
                    out.push(')');
                    last_atom = true;
                } else {
                    push_folded(&mut out, c, t.fold);
                    last_atom = true;
                }
            }
        }
        i += 1;
    }
    if in_class {
        return Err(format!("unterminated class in /{src}/"));
    }
    Ok((out, t.synth, t.renamed))
}

/// État partagé du traducteur : flags, tables de groupes.
#[allow(clippy::struct_excessive_bools)]
struct Trans {
    unicode: bool,
    /// JS-C625 : flag `v` — classes à grammaire étendue
    /// (`v_class` : `[` imbriqué, `&&`, `--`, `\q{…}`).
    vsets: bool,
    /// Flag `s` — `.` matche aussi les `LineTerminator`.
    dotall: bool,
    /// `i` en mode non-`u` : folding `ToUppercase` simple émis à la
    /// main (le `i` du backend est un folding Unicode complet).
    fold: bool,
    /// Nombre de groupes capturants du pattern (validation `\N`).
    n_groups: usize,
    /// Noms déclarés *avant* la position courante.
    names: std::collections::BTreeSet<String>,
    /// Tous les noms du pattern (backref avant déclaration).
    all_names: std::collections::BTreeSet<String>,
    /// Nom **émis** par numéro de groupe (0-indexé) — le nom
    /// déclaré pour `(?<x>`, `__eN` synthétique pour les anonymes
    /// quand `need_synth`. Alimente la réécriture `\N`→`\k<émis>`.
    emitted: Vec<String>,
    /// Vrai quand nommés + `\N` cohabitent : les anonymes sont
    /// renommés `(?P<__eN>` pour que le backend accepte les
    /// backrefs réécrites `\k<…>` (JS-C269).
    need_synth: bool,
    /// Noms synthétiques `__eN` émis — filtrés de `groups`/`named`
    /// à l'exec.
    synth: std::collections::BTreeSet<String>,
    /// JS-C624 : noms **renommés** `synth → orig` — fancy-regex
    /// n'accepte que `[A-Za-z0-9_]` pour `?P<…>` ; les
    /// `IdentifierName` non-ASCII (`(?<$𐒤>)`) sont émis sous un
    /// nom synthétique `__uN` et traduits à l'affichage.
    renamed: std::collections::BTreeMap<String, String>,
    /// Inverse de `renamed` — émission `\k<orig>` → `\k<synth>`.
    orig_to_synth: std::collections::BTreeMap<String, String>,
    /// Groupes capturants ouverts jusqu'ici (numérotation des `(`).
    groups_open: usize,
    /// Pile des groupes ouverts — `Some(n)` capturant, `None`
    /// sinon ; un `)` pop et clôt le numéro.
    open_stack: Vec<Option<usize>>,
    /// Parallèle à `open_stack` : `Some(pos_out)` pour les
    /// groupes *lookahead* (`(?=`/`(?!`) — position de `(?` dans
    /// `out`. Annex B autorise leur quantification (`(?=x)?`) que
    /// le backend refuse — le quantificateur est absorbé à la
    /// fermeture : min=0 → `(?:(?=…)|)`, min≥1 → forme nue
    /// (assertion idempotente) ; mesuré sur bbc.com (JS-C56).
    look_pos: Vec<Option<usize>>,
    /// Numéros de groupes **fermés** avant la position courante —
    /// seule une backref vers un groupe clos peut réellement
    /// capturer (`\3` dans `(\3)…(…)` matche vide, spec 21.2.2.9).
    closed: std::collections::BTreeSet<usize>,
}

impl Trans {
    /// Nom synthétique `__eN` pour le groupe anonyme `n` —
    /// garanti unique vis-à-vis des noms déclarés (`__eN` déjà
    /// pris → `__eN_`, etc.). Enregistré dans `emitted` et `synth`.
    fn synth_name(&mut self, n: usize) -> String {
        let mut nm = format!("__e{n}");
        while self.all_names.contains(&nm) || self.synth.contains(&nm) {
            nm.push('_');
        }
        self.synth.insert(nm.clone());
        self.emitted.push(nm.clone());
        nm
    }
}

/// Premier passage : tous les noms de groupes capturants + leur
/// nombre (les `\k<name>`/`\N` sont validés contre cette table —
/// une backref nommée peut précéder sa déclaration en JS) + la
/// présence d'un échappement `\`+chiffre hors classe (backref
/// décimale plausible — sur-approximation bénigne, JS-C269).
// ==================== JS-C625 : classes `v` ====================
//
// Sous-grammaire `ClassSetExpression` du flag `v` : classes
// imbriquées `[[…]]`, intersection `A&&B`, différence `A--B`,
// strings `\q{a|bc}`, ponctuateurs réservés (singles `(){}|/-`
// exigent l'échappement, doubles `&& !! ## … ~~` interdits).
// Abaissé vers fancy-regex : `A--B` → `(?![B])[A]`,
// `A&&B` → `(?=[B])[A]`, strings → alternance triée par
// longueur décroissante. `[^…]` porteur de strings : les
// strings 1-char rejoignent le corps nié, les multi-chars
// s'évanouissent — le complément fournit toujours un membre
// 1-char à la position (les strings exclues ≥2 chars ne
// peuvent jamais être consommées par l'atome nié).
/// Partie « 1 char » d'un set `v` évalué.
enum VPart {
    /// Aucun char dans le set.
    Empty,
    /// Corps de classe — fragments insérables dans `[…]`
    /// (fusionnable en union et opérande `&&`/`--` native).
    Body(String),
    /// Corps **nié** `[^…]` — atom `[^b]`, opérande `&&`
    /// imbriqué, non fusionnable en union.
    Neg(String),
    /// Atome complet (composite avec lookarounds — ne peut
    /// plus servir d'opérande de classe).
    Atom(String),
}

impl VPart {
    /// Le fragment atome consommant un char du set —
    /// toujours quantifiable (`[^\s\S]` pour le vide : une
    /// lookahead `(?!x)` n'est pas une cible de répétition
    /// valide côté backend).
    fn atom(&self) -> String {
        match self {
            VPart::Empty => "[^\\s\\S]".to_string(),
            VPart::Body(b) => format!("[{b}]"),
            VPart::Neg(b) => format!("[^{b}]"),
            VPart::Atom(a) => a.clone(),
        }
    }
    fn is_empty(&self) -> bool {
        matches!(self, VPart::Empty)
    }
}

/// Domaine des strings d'un set `v` (`\q{…}`).
enum VStrings {
    /// Strings énumérées — `(fragment échappé, len chars)`.
    Finite(Vec<(String, usize)>),
    /// Complément : toutes les strings sauf celles listées —
    /// produit d'une classe nichée niée porteuse de `\q{…}`.
    Complement(Vec<(String, usize)>),
}

/// Set `v` évalué — chars (part) + strings (`\q{…}`).
struct VSet {
    part: VPart,
    strings: VStrings,
}

impl VSet {
    fn empty() -> VSet {
        VSet {
            part: VPart::Empty,
            strings: VStrings::Finite(Vec::new()),
        }
    }
    /// `\q{…}` : une string de **1 char est un char** (même
    /// élément de set — `[\d&&\q{0}]` matche "0") → elle
    /// rejoint le `Body` ; les ≥2 restent des strings.
    fn with_strings(strings: Vec<(String, usize)>) -> VSet {
        let mut body = String::new();
        let mut strs = Vec::new();
        for (f, len) in strings {
            if len == 1 {
                body.push_str(&f);
            } else {
                strs.push((f, len));
            }
        }
        VSet {
            part: if body.is_empty() {
                VPart::Empty
            } else {
                VPart::Body(body)
            },
            strings: VStrings::Finite(strs),
        }
    }
}

/// Union de parties char : `Body`+`Body` concatène, toute
/// autre paire passe en alternance.
fn v_part_union(a: VPart, b: VPart) -> VPart {
    match (a, b) {
        (VPart::Empty, x) | (x, VPart::Empty) => x,
        (VPart::Body(mut x), VPart::Body(y)) => {
            x.push_str(&y);
            VPart::Body(x)
        }
        (a, b) => VPart::Atom(format!("(?:{}|{})", a.atom(), b.atom())),
    }
}

/// `A ∪ B` — union de strings : `Finite`+`Finite` concatène
/// dédup ; un `Complement` absorbe tout sauf ce qu'il exclut.
fn v_union(mut a: VSet, b: VSet) -> VSet {
    a.part = v_part_union(a.part, b.part);
    a.strings = match (a.strings, b.strings) {
        (VStrings::Finite(mut x), VStrings::Finite(y)) => {
            for s in y {
                if !x.iter().any(|z| z.0 == s.0) {
                    x.push(s);
                }
            }
            VStrings::Finite(x)
        }
        (VStrings::Complement(mut x), VStrings::Finite(y)) => {
            x.retain(|e| !y.iter().any(|z| z.0 == e.0));
            VStrings::Complement(x)
        }
        (VStrings::Finite(x), VStrings::Complement(mut y)) => {
            y.retain(|e| !x.iter().any(|z| z.0 == e.0));
            VStrings::Complement(y)
        }
        (VStrings::Complement(mut x), VStrings::Complement(y)) => {
            x.retain(|e| y.iter().any(|z| z.0 == e.0));
            VStrings::Complement(x)
        }
    };
    a
}

/// `A -- B` : `Body`/`Neg` descendent en syntaxe de classe
/// **native** (`a && [^b]` — quantifiable et fusionnable en
/// union) ; seul un `Atom` impose le repli lookahead
/// `(?![B])[A]` (non quantifiable côté backend — limite
/// connue). Strings : égalité exacte (un char ≠ sa string
/// 1-unit, spec).
fn v_diff(mut a: VSet, b: VSet) -> VSet {
    a.strings = match (a.strings, b.strings) {
        (VStrings::Finite(mut x), VStrings::Finite(y)) => {
            x.retain(|e| !y.iter().any(|z| z.0 == e.0));
            VStrings::Finite(x)
        }
        // B complément = « tout sauf excluded » — ne restent
        // de A que les strings que B exclut.
        (VStrings::Finite(mut x), VStrings::Complement(y)) => {
            x.retain(|e| y.iter().any(|z| z.0 == e.0));
            VStrings::Finite(x)
        }
        (s @ VStrings::Complement(_), _) => s,
    };
    if a.part.is_empty() || b.part.is_empty() {
        return a;
    }
    a.part = match (a.part, b.part) {
        // `a -- b` = `a ∩ ~b` → `a && [^b]` natif.
        (VPart::Body(x), VPart::Body(y)) => VPart::Body(format!("{x}&&[^{y}]")),
        // `a -- [^y]` = `a ∩ y` → `a && [y]`.
        (VPart::Body(x), VPart::Neg(y)) => VPart::Body(format!("{x}&&[{y}]")),
        // `[^x] -- y` = `~x ∩ ~y` = `~(x ∪ y)`.
        (VPart::Neg(x), VPart::Body(y)) => VPart::Neg(format!("{x}{y}")),
        // `[^x] -- [^y]` = `~x ∩ y` = `y && [^x]`.
        (VPart::Neg(x), VPart::Neg(y)) => VPart::Body(format!("{y}&&[^{x}]")),
        (a, b) => VPart::Atom(format!("(?!{}){}", b.atom(), a.atom())),
    };
    a
}

/// `A && B` : `Body`/`Neg` descendent en `a && [b]` natif ;
/// seul un `Atom` impose le repli `(?=[B])[A]`. Strings =
/// intersection exacte.
fn v_inter(mut a: VSet, b: VSet) -> VSet {
    a.strings = match (a.strings, b.strings) {
        (VStrings::Finite(mut x), VStrings::Finite(y)) => {
            x.retain(|e| y.iter().any(|z| z.0 == e.0));
            VStrings::Finite(x)
        }
        (VStrings::Finite(mut x), VStrings::Complement(y))
        | (VStrings::Complement(y), VStrings::Finite(mut x)) => {
            x.retain(|e| !y.iter().any(|z| z.0 == e.0));
            VStrings::Finite(x)
        }
        (VStrings::Complement(mut x), VStrings::Complement(y)) => {
            x.extend(y);
            VStrings::Complement(x)
        }
    };
    if a.part.is_empty() || b.part.is_empty() {
        a.part = VPart::Empty;
        return a;
    }
    a.part = match (a.part, b.part) {
        (VPart::Body(x), VPart::Body(y)) => VPart::Body(format!("{x}&&[{y}]")),
        (VPart::Body(x), VPart::Neg(y)) => VPart::Body(format!("{x}&&[^{y}]")),
        (VPart::Neg(x), VPart::Body(y)) => VPart::Body(format!("{y}&&[^{x}]")),
        // `~x ∩ ~y` = `~(x ∪ y)`.
        (VPart::Neg(x), VPart::Neg(y)) => VPart::Neg(format!("{x}{y}")),
        (a, b) => VPart::Atom(format!("(?={}){}", b.atom(), a.atom())),
    };
    a
}

/// Complément d'un set — opérande `[^…]` niché ou classe de
/// tête niée : chars → `Neg`/`Body`/`Atom`, strings → domaine
/// `Complement`.
fn v_negate_operand(set: VSet) -> VSet {
    // Le complément devient `Neg` (`[^…]` : atom `[^b]`,
    // opérande `&&`/`--` imbriqué) — pas un `Body` : concaténé
    // dans une union, le `^` tomberait en milieu de classe
    // (`[^0-9][a]` émis `[^0-9a]` = complément de l'union ≠
    // union des compléments). Les strings 1-char ne touchent
    // pas le domaine chars : chars et strings sont des
    // éléments *distincts* (spec) — `[^\q{a}x]` matche 'a'.
    let part = match &set.part {
        // Complément du vide = tout char — `Body` fusionnable.
        VPart::Empty => VPart::Body("\\s\\S".to_string()),
        VPart::Body(b) => VPart::Neg(b.clone()),
        VPart::Neg(n) => VPart::Body(n.clone()),
        // Atome composite : `(?!A)[\s\S]`.
        VPart::Atom(a) => VPart::Atom(format!("(?:(?!{a})[\\s\\S])")),
    };
    let strings = match &set.strings {
        VStrings::Finite(ss) => VStrings::Complement(ss.clone()),
        VStrings::Complement(ss) => VStrings::Finite(ss.clone()),
    };
    VSet { part, strings }
}

/// Char littéral en position pattern — les métachars fancy
/// sont échappés (`\q{…}` assemble des fragments).
fn v_lit_char(c: char) -> String {
    match c {
        '\\' | '.' | '^' | '$' | '*' | '+' | '?' | '(' | ')' | '[' | ']' | '{' | '}' | '|'
        | '/' => format!("\\{c}"),
        _ => c.to_string(),
    }
}

/// Strings `\p{Emoji_Keycap_Sequence}` — propriété « de
/// strings » finie (12 membres) non connue du backend.
fn emoji_keycap_strings() -> Vec<(String, usize)> {
    // `*`/`#` échappés via `v_lit_char` — `|*\u{fe0f}…` nu =
    // quantificateur sans cible côté backend.
    "0123456789#*"
        .chars()
        // Format `\x{…}` aligné sur `escape()` — l'intersection/
        // différence de strings compare les fragments textuellement.
        .map(|c| (format!("{}\\x{{fe0f}}\\x{{20e3}}", v_lit_char(c)), 3))
        .collect()
}

/// Opérateur d'ensemble rencontré entre opérandes.
#[derive(Clone, Copy, PartialEq)]
enum VOp {
    Inter,
    Diff,
}

/// Parse `\q{…}` — items `|`-séparés de chars et
/// `CharacterEscape`s. `i` = index du `\`.
fn v_q_strings(
    t: &mut Trans,
    cs: &[char],
    i: usize,
    src: &str,
) -> Result<(Vec<(String, usize)>, usize), String> {
    let mut j = i + 3; // après `\q{`
    let mut strings = Vec::new();
    let mut frag = String::new();
    let mut len = 0usize;
    loop {
        match cs.get(j) {
            None => return Err(format!("unterminated \\q{{…}} in /{src}/")),
            Some('}') => {
                strings.push((frag, len));
                return Ok((strings, j + 1));
            }
            Some('|') => {
                strings.push((std::mem::take(&mut frag), len));
                len = 0;
                j += 1;
            }
            Some('\\') => {
                // `\q{…}` : `CharacterEscape` seulement — les
                // `CharacterClassEscape` (`\d`, `\s`, `\w`,
                // `\p{…}` et négations) sont un early error.
                if let Some('d' | 'D' | 's' | 'S' | 'w' | 'W' | 'p' | 'P' | 'b' | 'B') =
                    cs.get(j + 1)
                {
                    return Err(format!("class escape in \\q{{…}} of /{src}/"));
                }
                let (f, next) = t.escape(cs, j, src)?;
                len += 1;
                frag.push_str(&f);
                j = next;
            }
            Some(&c) => {
                frag.push_str(&v_lit_char(c));
                len += 1;
                j += 1;
            }
        }
    }
}

/// Parse un `ClassSetOperand` : `[…]` niché, `\q{…}`,
/// `CharacterClassEscape`, `ClassSetCharacter`. Retourne
/// `(set, char unique si opérande char, index suivant)`.
fn v_operand(
    t: &mut Trans,
    cs: &[char],
    i: usize,
    src: &str,
) -> Result<(VSet, Option<u32>, usize), String> {
    match cs.get(i) {
        Some('[') => {
            // Classe nichée — récursion complète (`^` possible).
            let (set, next) = v_set(t, cs, i, src)?;
            Ok((set, None, next))
        }
        Some('\\') => {
            if cs.get(i + 1) == Some(&'q') {
                if cs.get(i + 2) == Some(&'{') {
                    let (ss, next) = v_q_strings(t, cs, i, src)?;
                    return Ok((VSet::with_strings(ss), None, next));
                }
                return Err(format!("malformed \\q in `v` regex /{src}/"));
            }
            if matches!(cs.get(i + 1), Some('p' | 'P')) && cs.get(i + 2) == Some(&'{') {
                // `\p{Emoji_Keycap_Sequence}` — strings finies.
                let body: String = cs[i + 3..].iter().take_while(|&&c| c != '}').collect();
                if body == "Emoji_Keycap_Sequence" && cs[i + 1] == 'p' {
                    return Ok((
                        VSet::with_strings(emoji_keycap_strings()),
                        None,
                        i + 3 + body.len() + 1,
                    ));
                }
            }
            let (frag, next) = t.escape(cs, i, src)?;
            // JS-C625 : `\s` JS inclut FEFF (mémo traduction).
            let frag = if frag == "\\s" {
                "\\s\\x{feff}".to_string()
            } else {
                frag
            };
            let cp = frag_char(&frag).map(u32::from);
            Ok((
                VSet {
                    part: VPart::Body(frag),
                    strings: VStrings::Finite(Vec::new()),
                },
                cp,
                next,
            ))
        }
        Some(&c) => {
            // Réservés `v` : singles `(){}|/-` exigent
            // l'échappement (`[`/`]` gérés en amont) ; les
            // doubles ponctuateurs `!! ## … ~~` sont interdits.
            const DP: &str = "&!#$%*+,.:;<=>?@^`~";
            match c {
                '(' | ')' | '{' | '}' | '/' | '|' => {
                    return Err(format!("unescaped `{c}` in `v` class /{src}/"));
                }
                '-' => {
                    return Err(format!("unescaped `-` in `v` class /{src}/"));
                }
                _ => {}
            }
            if DP.contains(c) && cs.get(i + 1) == Some(&c) {
                return Err(format!("reserved `{c}{c}` in `v` class /{src}/"));
            }
            let mut body = String::new();
            class_char(&mut body, u32::from(c));
            Ok((
                VSet {
                    part: VPart::Body(body),
                    strings: VStrings::Finite(Vec::new()),
                },
                Some(u32::from(c)),
                i + 1,
            ))
        }
        None => Err(format!("unterminated `v` class in /{src}/")),
    }
}

/// Parse `ClassSetExpression` jusqu'au `]` fermant —
/// retourne le set et l'index du `]`. `i` = premier membre.
fn v_expr(t: &mut Trans, cs: &[char], i: usize, src: &str) -> Result<(VSet, usize), String> {
    let mut acc = VSet::empty();
    let mut op: Option<VOp> = None;
    let mut members = 0usize;
    // Opérateur consommé : un opérande est exigé ensuite
    // (`[a&&]`/`[a&&b c]`/`[&&a]` → SyntaxError).
    let mut pending_op = false;
    let mut j = i;
    loop {
        match cs.get(j) {
            None => return Err(format!("unterminated `v` class in /{src}/")),
            Some(']') => {
                if pending_op {
                    return Err(format!("missing `v` operand in /{src}/"));
                }
                return Ok((acc, j));
            }
            Some('&') if cs.get(j + 1) == Some(&'&') => {
                // `&&` en tête, après ≥2 membres juxtaposés
                // (union + op — hors grammaire) ou en chaîne `--`.
                if op == Some(VOp::Diff) || (op.is_none() && members != 1) {
                    return Err(format!("mixed `&&` in `v` class /{src}/"));
                }
                pending_op = true;
                op = Some(VOp::Inter);
                j += 2;
            }
            Some('-') if cs.get(j + 1) == Some(&'-') => {
                if op == Some(VOp::Inter) || (op.is_none() && members != 1) {
                    return Err(format!("mixed `--` in `v` class /{src}/"));
                }
                pending_op = true;
                op = Some(VOp::Diff);
                j += 2;
            }
            _ => {
                if op.is_some() && !pending_op {
                    return Err(format!("mixed operands in `v` class /{src}/"));
                }
                pending_op = false;
                let (operand, single_cp, next) = v_operand(t, cs, j, src)?;
                // `a-b` : range — uniquement en union, jamais
                // en opérande d'`&&`/`--` (spec → SyntaxError).
                if op.is_none()
                    && single_cp.is_some()
                    && cs.get(next) == Some(&'-')
                    && cs.get(next + 1) != Some(&'-')
                {
                    let (_bound, bcp, bnext) = v_operand(t, cs, next + 1, src)?;
                    let Some(hi) = bcp else {
                        return Err(format!("class range bound in `v` /{src}/"));
                    };
                    let lo = single_cp.unwrap_or(0);
                    if lo > hi {
                        return Err(format!("invalid class range in `v` /{src}/"));
                    }
                    let mut body = String::new();
                    class_char(&mut body, lo);
                    body.push('-');
                    class_char(&mut body, hi);
                    acc = v_union(
                        acc,
                        VSet {
                            part: VPart::Body(body),
                            strings: VStrings::Finite(Vec::new()),
                        },
                    );
                    members += 1;
                    j = bnext;
                    continue;
                }
                acc = match op {
                    Some(VOp::Inter) => v_inter(acc, operand),
                    Some(VOp::Diff) => v_diff(acc, operand),
                    None => v_union(acc, operand),
                };
                members += 1;
                j = next;
            }
        }
    }
}

/// Parse `[…]` `v` complet (négation `^` incluse) — rend le
/// **set** (pour usage d'opérande niché). `i` = index du `[`.
fn v_set(t: &mut Trans, cs: &[char], i: usize, src: &str) -> Result<(VSet, usize), String> {
    let mut j = i + 1;
    let neg = cs.get(j) == Some(&'^');
    if neg {
        j += 1;
    }
    let (set, k) = v_expr(t, cs, j, src)?;
    if cs.get(k) != Some(&']') {
        return Err(format!("unterminated `v` class in /{src}/"));
    }
    let set = if neg { v_negate_operand(set) } else { set };
    Ok((set, k + 1))
}

/// Assemble l'atome pattern final : `[corps]` seul, atome
/// composite, ou alternance `strings | chars` (longueur décroissante).
fn v_assemble(set: VSet) -> String {
    let mut strs = match &set.strings {
        VStrings::Finite(ss) => ss.clone(),
        // Complément de strings au niveau atom : le complément
        // fournit toujours un membre 1-char à la position —
        // approximation `[\s\S]` couverte par la partie char
        // niée (JS-C625, limite connue).
        VStrings::Complement(_) => Vec::new(),
    };
    strs.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    let mut salt = strs
        .iter()
        .map(|(f, _)| f.as_str())
        .collect::<Vec<_>>()
        .join("|");
    // `\q{}` — string vide : alternative terminale vide.
    if strs.iter().any(|(f, _)| f.is_empty()) {
        salt.push('|');
    }
    if salt.is_empty() {
        return match &set.part {
            // `[^\s\S]` — classe vide **quantifiable** (une
            // lookahead `(?!x)` n'est pas une cible valide).
            VPart::Empty => "[^\\s\\S]".to_string(),
            VPart::Body(b) => format!("[{b}]"),
            VPart::Neg(b) => format!("[^{b}]"),
            // `(?:…)` — un quantificateur se lie à l'atome
            // entier, pas à la dernière alternative/lookahead.
            VPart::Atom(a) => format!("(?:{a})"),
        };
    }
    match &set.part {
        VPart::Empty => format!("(?:{salt})"),
        VPart::Body(b) => format!("(?:{salt}|[{b}])"),
        VPart::Neg(b) => format!("(?:{salt}|[^{b}])"),
        VPart::Atom(a) => format!("(?:{salt}|{a})"),
    }
}

/// Point d'entrée — `[…]` en mode `v`. `i` = index du `[` ;
/// retourne `(atome pattern, index après le `]` fermant)`.
fn v_class(t: &mut Trans, cs: &[char], i: usize, src: &str) -> Result<(String, usize), String> {
    let (set, next) = v_set(t, cs, i, src)?;
    Ok((v_assemble(set), next))
}

fn prescan(cs: &[char]) -> (std::collections::BTreeSet<String>, usize, bool) {
    let mut all = std::collections::BTreeSet::new();
    let mut n = 0usize;
    let mut has_decimal = false;
    let mut in_class = false;
    let mut i = 0;
    while i < cs.len() {
        match cs[i] {
            '\\' => {
                if !in_class && cs.get(i + 1).is_some_and(char::is_ascii_digit) {
                    has_decimal = true;
                }
                i += 1;
            }
            '[' if !in_class => in_class = true,
            ']' if in_class => in_class = false,
            '(' if !in_class => {
                if cs.get(i + 1) == Some(&'?') {
                    if cs.get(i + 2) == Some(&'<') && !matches!(cs.get(i + 3), Some('=' | '!')) {
                        let mut j = i + 3;
                        let mut name = String::new();
                        while let Some(&d) = cs.get(j) {
                            if d == '>' {
                                break;
                            }
                            name.push(d);
                            j += 1;
                        }
                        if cs.get(j) == Some(&'>') && valid_group_name(&name) {
                            all.insert(name);
                            n += 1;
                        }
                    }
                } else {
                    n += 1;
                }
            }
            _ => {}
        }
        i += 1;
    }
    (all, n, has_decimal)
}

/// Fragment d'échappement hors classe en mode fold : littéral →
/// `[cC]`, backref → `(?i:…)` (la comparaison reste insensible).
fn emit_escape(out: &mut String, frag: &str, fold: bool) {
    if !fold {
        out.push_str(frag);
    } else if let Some(c) = frag_char(frag) {
        push_folded(out, c, true);
    } else if frag.starts_with("\\k<") || frag.as_bytes().get(1).is_some_and(u8::is_ascii_digit) {
        out.push_str("(?i:");
        out.push_str(frag);
        out.push(')');
    } else {
        out.push_str(frag);
    }
}

/// Char littéral hors classe : `[cCU]` quand le char a une
/// variante de casse (sémantique `i` non-`u` : `Canonicalize`
/// compare les `ToUppercase`, donc 'A' matche aussi 'a').
fn push_folded(out: &mut String, c: char, fold: bool) {
    if fold {
        let variants: Vec<u32> = fold_variants(u32::from(c)).collect();
        if !variants.is_empty() {
            out.push('[');
            class_char(out, u32::from(c));
            for v in variants {
                class_char(out, v);
            }
            out.push(']');
            return;
        }
    }
    out.push(c);
}

/// Variantes de casse d'un char pour le fold non-`u` : uppercase
/// et lowercase simples *dont la canonisation égale celle de `cp`*
/// (multi-chars ignorés). La garde exclut `lower(KELVIN)='k'` :
/// `upper('k')='K' ≠ KELVIN` → `/\u{212a}/i` ne matche pas 'k'.
fn fold_variants(cp: u32) -> impl Iterator<Item = u32> {
    let canon = simple_upper(cp).unwrap_or(cp);
    [simple_upper(cp), simple_lower(cp)]
        .into_iter()
        .flatten()
        .filter(move |&v| simple_upper(v).unwrap_or(v) == canon)
}

/// Émet les variantes de casse de `cp` comme items de classe.
fn emit_fold_variants(out: &mut String, cp: u32) {
    for v in fold_variants(cp) {
        class_char(out, v);
    }
}

/// `Canonicalize` JS non-`u` : `ToUppercase` retenu seulement s'il
/// produit un char unique différent (ß → `SS` est ignoré).
fn simple_upper(cp: u32) -> Option<u32> {
    let c = char::from_u32(cp)?;
    let mut it = c.to_uppercase();
    let u = it.next()?;
    (it.next().is_none() && u != c).then_some(u32::from(u))
}

/// Miroir de [`simple_upper`] pour `ToLowercase`.
fn simple_lower(cp: u32) -> Option<u32> {
    let c = char::from_u32(cp)?;
    let mut it = c.to_lowercase();
    let u = it.next()?;
    (it.next().is_none() && u != c).then_some(u32::from(u))
}

/// Char sûr à injecter dans une classe `[…]` backend.
fn class_char(out: &mut String, cp: u32) {
    match char::from_u32(cp) {
        Some(c) if c.is_ascii_alphanumeric() => out.push(c),
        _ => out.push_str(&format!("\\x{{{cp:x}}}")),
    }
}

/// Le fragment émis par `escape` est-il un char littéral unique ?
fn frag_char(frag: &str) -> Option<char> {
    let mut cs = frag.chars();
    if let (Some(c), None) = (cs.next(), cs.next()) {
        return Some(c);
    }
    let hex = frag.strip_prefix("\\x{")?.strip_suffix('}')?;
    char::from_u32(u32::from_str_radix(hex, 16).ok()?)
}

/// Atome de classe vu par le validateur de ranges.
#[derive(Clone, Copy)]
enum ClassAtom {
    /// Literal char (escape ou brut).
    Char(u32),
    /// `\d`/`\s`/`\w` et négations — ne peut borner une range `u`.
    Escape,
    /// Fin de classe / EOF — `-` devient littéral.
    End,
}

/// Échappe décimal **en classe** (Annex B — jamais une
/// backref) : `\0`+≤2 octaux, `\1`–`\7`+≤2 octaux (3 max),
/// `\8`/`\9` identité. Retourne `(valeur, index après)`.
/// `\18` = `\x{1}` puis `8` littéral (la consommation
/// s'arrête au premier non-octal) — §B-56 bras 3 : la
/// voie `escape()`/`decimal_escape` lisait `\N` comme
/// backref et émettait `\1(?:00)` dans la classe →
/// « Invalid character class » (jscrambler airfrance vol 101).
fn class_octal(cs: &[char], i: usize) -> (u32, usize) {
    debug_assert_eq!(cs.get(i), Some(&'\\'));
    let d = cs[i + 1];
    if d == '0' {
        return if cs.get(i + 2).is_some_and(char::is_ascii_digit) {
            let (mut v, mut j, mut nd) = (0u32, i + 2, 0);
            while nd < 2 {
                match cs.get(j).and_then(|x| x.to_digit(8)) {
                    Some(o) => {
                        v = v * 8 + o;
                        j += 1;
                        nd += 1;
                    }
                    None => break,
                }
            }
            (v, j)
        } else {
            (0, i + 2)
        };
    }
    if matches!(d, '8' | '9') {
        return (u32::from(d), i + 2);
    }
    let (mut v, mut j, mut nd) = (0u32, i + 1, 0);
    while nd < 3 {
        match cs.get(j).and_then(|x| x.to_digit(8)) {
            Some(o) => {
                v = v * 8 + o;
                j += 1;
                nd += 1;
            }
            None => break,
        }
    }
    (v, j)
}

/// Classifie l'atome de classe à `cs[i]` (sans l'émettre) —
/// décode les échappements comme `escape()` : `\xHH`, `\uHHHH`
/// (paires de surrogates fusionnées), `\u{…}`, `\0`/octales
/// Annex B, `\cX`. `\d`/`\p`/`\k<…>` → `Escape`.
fn class_atom_of(cs: &[char], i: usize) -> ClassAtom {
    match cs.get(i) {
        None | Some(']') => ClassAtom::End,
        Some('\\') => match cs.get(i + 1) {
            Some('d' | 'D' | 's' | 'S' | 'w' | 'W' | 'p' | 'P') => ClassAtom::Escape,
            Some('b') => ClassAtom::Char(8), // `\b` in class = backspace
            Some('n') => ClassAtom::Char(10),
            Some('r') => ClassAtom::Char(13),
            Some('t') => ClassAtom::Char(9),
            Some('f') => ClassAtom::Char(12),
            Some('v') => ClassAtom::Char(11),
            Some('x') => match (
                cs.get(i + 2).and_then(|d| d.to_digit(16)),
                cs.get(i + 3).and_then(|d| d.to_digit(16)),
            ) {
                (Some(a), Some(b)) => ClassAtom::Char(a * 16 + b),
                // `\x` malformé sloppy → identité `x`.
                _ => ClassAtom::Char(u32::from('x')),
            },
            Some('u') => {
                if cs.get(i + 2) == Some(&'{') {
                    let mut j = i + 3;
                    let mut v = 0u32;
                    let mut digits = 0;
                    while let Some(&d) = cs.get(j) {
                        if let Some(h) = d.to_digit(16) {
                            v = v.saturating_mul(16).saturating_add(h);
                            digits += 1;
                            j += 1;
                        } else {
                            break;
                        }
                    }
                    if cs.get(j) == Some(&'}') && digits > 0 {
                        return ClassAtom::Char(v);
                    }
                    return ClassAtom::Char(u32::from('u')); // sloppy identité
                }
                // Paire de surrogates → point de code fusionné.
                if let Some((cp, _)) = surrogate_pair(cs, i) {
                    return ClassAtom::Char(u32::from(cp));
                }
                match hex4(cs, i + 2) {
                    Some(v) => ClassAtom::Char(v),
                    None => ClassAtom::Char(u32::from('u')),
                }
            }
            // `\0` NUL ; `\0N` → octale Annex B (`\07`).
            Some('0') => {
                if cs.get(i + 2).is_some_and(char::is_ascii_digit) {
                    let mut v = 0u32;
                    let mut j = i + 2;
                    let mut nd = 0;
                    while nd < 2 {
                        match cs.get(j).and_then(|d| d.to_digit(8)) {
                            Some(o) => {
                                v = v * 8 + o;
                                j += 1;
                                nd += 1;
                            }
                            None => break,
                        }
                    }
                    ClassAtom::Char(v)
                } else {
                    ClassAtom::Char(0)
                }
            }
            Some('c') => match cs.get(i + 2).filter(|d| d.is_ascii_alphabetic()) {
                Some(&d) => ClassAtom::Char(u32::from(d) % 32),
                // `\c` non suivi d'une lettre → `\\c` émis :
                // la borne vue est le backslash.
                None => ClassAtom::Char(u32::from('\\')),
            },
            // `\1`–`\7` : dans une classe les backrefs sont des
            // octales Annex B ; `\8`/`\9` → identité.
            Some(&d) if d.is_ascii_digit() => {
                if matches!(d, '8' | '9') {
                    ClassAtom::Char(u32::from(d))
                } else {
                    let mut v = 0u32;
                    let mut j = i + 1;
                    let mut nd = 0;
                    while nd < 3 {
                        match cs.get(j).and_then(|x| x.to_digit(8)) {
                            Some(o) => {
                                v = v * 8 + o;
                                j += 1;
                                nd += 1;
                            }
                            None => break,
                        }
                    }
                    ClassAtom::Char(v)
                }
            }
            // `\k` en classe : identité Annex B (les backrefs
            // nommées ne vivent pas dans les classes).
            Some('k') => ClassAtom::Char(u32::from('k')),
            Some(&c) => ClassAtom::Char(u32::from(c)),
            None => ClassAtom::End,
        },
        Some(&c) => ClassAtom::Char(u32::from(c)),
    }
}

/// `cp` est-il un surrogate — jamais présent dans nos strings
/// UTF-8 : un tel atome ne peut matcher aucun caractère.
fn is_surrogate(cp: u32) -> bool {
    (0xd800..=0xdfff).contains(&cp)
}

/// `[\uDClo]` ou `[\uDClo-\uDChi]` à `cs[i]` après une surrogate
/// haute `hi` → fragment de classe astrale fusionnée.
/// Forme purement surrogate uniquement (un autre atome dans la
/// classe, `^` ou un `\x` ferait retomber sur `(?!)` + classe
/// normale — approximation documentée).
fn astral_class(cs: &[char], i: usize, hi: u32, unicode: bool) -> Option<(String, usize)> {
    if cs.get(i) != Some(&'[') || cs.get(i + 1) == Some(&'^') {
        return None;
    }
    let low = |at: usize| -> Option<u32> {
        if cs.get(at) == Some(&'\\') && cs.get(at + 1) == Some(&'u') {
            hex4(cs, at + 2).filter(|v| (0xdc00..0xe000).contains(v))
        } else {
            None
        }
    };
    let mut j = i + 1;
    let mut lo = low(j)?;
    j += 6;
    let base = 0x1_0000 + ((hi - 0xd800) << 10);
    if cs.get(j) == Some(&']') {
        // `[\uDClo]` — un seul point astral.
        if unicode {
            return Some((format!("\\x{{{:x}}}", base + (lo - 0xdc00)), j + 1));
        }
        // JS-C622 non-`u` : domaine units éclatés — sentinelle
        // lead + sentinelle trail.
        return Some((format!("{}{}", sent_frag(hi), sent_frag(lo)), j + 1));
    }
    // Classe multi-items de basses surrogates — atomes `\uDCxx`
    // et plages `\uDCxx-\uDCyy` fusionnés en ensemble astral
    // (`\uD800[\uDD00-\uDD02\uDF9F\uDFD0]`, bundle kevlar
    // YouTube). Tout autre atome → `None` (retombée `(?!)` +
    // classe normale).
    let mut frag = if unicode {
        String::from("[")
    } else {
        // JS-C622 non-`u` : lead sentinelle devant la classe de
        // sentinelles trail.
        format!("{}[", sent_frag(hi))
    };
    loop {
        if cs.get(j) == Some(&'-') {
            let hi2 = low(j + 1)?;
            let (a, b) = if unicode {
                (base + (lo - 0xdc00), base + (hi2 - 0xdc00))
            } else {
                // JS-C622 non-`u` : membres = sentinelles trail.
                (0xF_0000 + lo - 0xD800, 0xF_0000 + hi2 - 0xD800)
            };
            frag.push_str(&format!("\\x{{{a:x}}}-\\x{{{b:x}}}"));
            j += 7;
        } else {
            // JS-C622 non-`u` : membre = sentinelle trail.
            let a = if unicode {
                base + (lo - 0xdc00)
            } else {
                0xF_0000 + lo - 0xD800
            };
            frag.push_str(&format!("\\x{{{a:x}}}"));
        }
        match cs.get(j) {
            Some(&']') => {
                frag.push(']');
                return Some((frag, j + 1));
            }
            Some(&'\\') => {
                lo = low(j)?;
                j += 6;
            }
            _ => return None,
        }
    }
}

/// `{n}` / `{n,}` / `{n,m}` à `cs[i]` → `(lo, hi, next)`.
fn parse_quantifier(cs: &[char], i: usize) -> Option<(u32, Option<u32>, usize)> {
    if cs.get(i) != Some(&'{') {
        return None;
    }
    let mut j = i + 1;
    let mut lo = 0u32;
    let mut digits = 0;
    while let Some(&d) = cs.get(j) {
        if let Some(v) = d.to_digit(10) {
            lo = lo.checked_mul(10)?.checked_add(v)?;
            digits += 1;
            j += 1;
        } else {
            break;
        }
    }
    if digits == 0 {
        return None;
    }
    match cs.get(j) {
        Some('}') => Some((lo, Some(lo), j + 1)),
        Some(',') => {
            j += 1;
            let mut hi = 0u32;
            let mut hdigits = 0;
            while let Some(&d) = cs.get(j) {
                if let Some(v) = d.to_digit(10) {
                    hi = hi.checked_mul(10)?.checked_add(v)?;
                    hdigits += 1;
                    j += 1;
                } else {
                    break;
                }
            }
            if cs.get(j) != Some(&'}') {
                return None;
            }
            Some((lo, (hdigits > 0).then_some(hi), j + 1))
        }
        _ => None,
    }
}

/// `(?…)` à `cs[i]` — validation JS + traduction. Retourne le
/// prochain index (juste après la tête de groupe).
fn translate_group(
    t: &mut Trans,
    cs: &[char],
    i: usize,
    out: &mut String,
    src: &str,
) -> Result<usize, String> {
    let err = || format!("invalid group in /{src}/");
    match cs.get(i + 2) {
        Some(':' | '=' | '!') => {
            t.open_stack.push(None);
            // `=`/`!` → lookahead quantifiable (Annex B) ; `:` non.
            t.look_pos.push(if cs[i + 2] == ':' {
                None
            } else {
                Some(out.len())
            });
            out.push_str("(?");
            out.push(cs[i + 2]);
            Ok(i + 3)
        }
        Some('<') if matches!(cs.get(i + 3), Some('=' | '!')) => {
            t.open_stack.push(None);
            t.look_pos.push(None);
            out.push_str("(?<");
            out.push(cs[i + 3]);
            Ok(i + 4)
        }
        Some('<') => {
            // Groupe nommé : `name` = IdentifierName, unique.
            let mut j = i + 3;
            let mut name = String::new();
            while let Some(&c) = cs.get(j) {
                if c == '>' {
                    break;
                }
                name.push(c);
                j += 1;
            }
            if cs.get(j) != Some(&'>') {
                return Err(err());
            }
            if !valid_group_name(&name) {
                return Err(err());
            }
            if !t.names.insert(name.clone()) {
                return Err(format!("duplicate group name `{name}` in /{src}/"));
            }
            t.groups_open += 1;
            t.open_stack.push(Some(t.groups_open));
            t.look_pos.push(None);
            // JS-C624 : fancy-regex n'accepte que `[A-Za-z0-9_]`
            // dans `?P<…>` — un IdentifierName plus large (`$`,
            // non-ASCII, non-BMP) est émis sous un nom synthétique
            // `__uN` (dédoublé) et traduit à l'affichage.
            let emit_name = if name_needs_rename(&name) {
                let mut nm = format!("__u{}", t.renamed.len());
                while t.all_names.contains(&nm)
                    || t.synth.contains(&nm)
                    || t.renamed.contains_key(&nm)
                {
                    nm.push('_');
                }
                t.renamed.insert(nm.clone(), name.clone());
                t.orig_to_synth.insert(name, nm.clone());
                nm
            } else {
                name
            };
            t.emitted.push(emit_name.clone());
            out.push_str("(?P<");
            out.push_str(&emit_name);
            out.push('>');
            Ok(j + 1)
        }
        Some('P' | '#') => Err(err()),
        _ => {
            // Modificateurs inline ES2025 : `(?ims:…)` ou
            // `(?ims-ims:…)` — `ims` seuls, pas de doublon, les
            // deux côtés disjoints, `:` obligatoire.
            let mut j = i + 2;
            let mut add = Vec::new();
            while let Some(&f) = cs.get(j) {
                if matches!(f, 'i' | 'm' | 's') && !add.contains(&f) {
                    add.push(f);
                    j += 1;
                } else {
                    break;
                }
            }
            let mut sub = Vec::new();
            let mut saw_dash = false;
            if cs.get(j) == Some(&'-') {
                saw_dash = true;
                j += 1;
                while let Some(&f) = cs.get(j) {
                    if matches!(f, 'i' | 'm' | 's') && !sub.contains(&f) {
                        sub.push(f);
                        j += 1;
                    } else {
                        break;
                    }
                }
            }
            if cs.get(j) != Some(&':')
                || (add.is_empty() && !saw_dash)
                || add.iter().any(|f| sub.contains(f))
            {
                return Err(err());
            }
            // Fragment validé → émis tel quel (fancy le comprend).
            t.open_stack.push(None);
            t.look_pos.push(None);
            out.push_str(&cs[i..=j].iter().collect::<String>());
            Ok(j + 1)
        }
    }
}

/// JS-C624 : fancy-regex n'accepte que `[A-Za-z0-9_]` dans les
/// noms `?P<…>` — tout `IdentifierName` plus large (`$`, lettres
/// non-ASCII, non-BMP) part en renommage synthétique `__uN`.
fn name_needs_rename(name: &str) -> bool {
    name.chars().any(|c| !c.is_ascii_alphanumeric() && c != '_')
}

/// `name` est un `IdentifierName` (tables UAX #31 de
/// `unicode-ident` + `Other_ID_*`, `$`/`_`/ZWNJ/ZWJ — les mêmes
/// règles que le lexer).
fn valid_group_name(name: &str) -> bool {
    let mut cs = name.chars();
    match cs.next() {
        Some(c) if crate::lexer::is_ident_start_pub(c) => {}
        _ => return false,
    }
    cs.all(crate::lexer::is_ident_part_pub)
}

impl Trans {
    /// Un échappement `\…` à `cs[i]` → fragment natif + validation.
    /// Retourne `(fragment, next_index)` ou une erreur `SyntaxError`.
    /// Le fragment est toujours *entier* (`\xNN`, `\uHHHH`…
    /// consommés) pour que le folding `i` non-`u` puisse décider.
    fn escape(&self, cs: &[char], i: usize, src: &str) -> Result<(String, usize), String> {
        let Some(&c) = cs.get(i + 1) else {
            return Ok(("\\".into(), i + 1));
        };
        // `\u{hex}` — les deux modes ; valeur ≤ 0x10FFFF, zéros de
        // tête normalisés (`\u{000000003f}` = '?').
        if c == 'u' && cs.get(i + 2) == Some(&'{') {
            let mut j = i + 3;
            let mut v = 0u64;
            let mut digits = 0;
            while let Some(&d) = cs.get(j) {
                if let Some(h) = d.to_digit(16) {
                    v = v.saturating_mul(16).saturating_add(u64::from(h));
                    digits += 1;
                    j += 1;
                } else {
                    break;
                }
            }
            if cs.get(j) == Some(&'}') && digits > 0 {
                if v > 0x10_FFFF {
                    return Err(format!("\\u{{…}} out of range in /{src}/"));
                }
                if !self.unicode && v > 0xFFFF {
                    // JS-C622 : astral non-`u` = paire de
                    // sentinelles (domaine units éclatés).
                    let a = v as u32 - 0x1_0000;
                    return Ok((
                        format!(
                            "{}{}",
                            sent_frag(0xD800 + (a >> 10)),
                            sent_frag(0xDC00 + (a & 0x3FF))
                        ),
                        j + 1,
                    ));
                }
                return Ok((format!("\\x{{{v:x}}}"), j + 1));
            }
            if self.unicode {
                return Err(format!("malformed \\u{{…}} in `u` regex /{src}/"));
            }
            // Sloppy : `\u{` malformé retombe sur l'identité `u`.
            return Ok(("u".into(), i + 2));
        }
        if c == 'u' {
            // Mode `u` : paire de surrogates fusionnée en un point.
            if self.unicode {
                if let Some((cp, next)) = surrogate_pair(cs, i) {
                    return Ok((format!("\\x{{{:x}}}", u32::from(cp)), next));
                }
            }
            if let Some(v) = hex4(cs, i + 2) {
                return Ok((format!("\\x{{{v:x}}}"), i + 6));
            }
            if self.unicode {
                return Err(format!("malformed \\u in `u` regex /{src}/"));
            }
            return Ok(("u".into(), i + 2));
        }
        if c == 'x' {
            if let (Some(a), Some(b)) = (
                cs.get(i + 2).and_then(|d| d.to_digit(16)),
                cs.get(i + 3).and_then(|d| d.to_digit(16)),
            ) {
                return Ok((format!("\\x{{{:x}}}", a * 16 + b), i + 4));
            }
            if self.unicode {
                return Err(format!("malformed \\x in `u` regex /{src}/"));
            }
            return Ok(("x".into(), i + 2));
        }
        if c == 'k' {
            // `\k` n'est une identity escape Annex B qu'en sloppy
            // SANS aucun groupe nommé dans le pattern ; sinon la
            // forme `\k<name>` est exigée (malformée → SyntaxError).
            let strict_k = self.unicode || !self.all_names.is_empty();
            if cs.get(i + 2) == Some(&'<') {
                let mut j = i + 3;
                let mut name = String::new();
                while let Some(&d) = cs.get(j) {
                    if d == '>' {
                        break;
                    }
                    name.push(d);
                    j += 1;
                }
                if cs.get(j) == Some(&'>') && valid_group_name(&name) {
                    if self.names.contains(&name) {
                        // JS-C624 : groupe renommé → `\k<synth>`.
                        let nm = self
                            .orig_to_synth
                            .get(&name)
                            .map_or(name.as_str(), String::as_str);
                        return Ok((format!("\\k<{nm}>"), j + 1));
                    }
                    if self.all_names.contains(&name) {
                        // Référence avant déclaration : le groupe ne
                        // peut pas avoir capturé ici → la backref
                        // matche la chaîne vide (spec 21.2.2.9).
                        return Ok(("(?:)".into(), j + 1));
                    }
                    if strict_k {
                        return Err(format!("unknown group `{name}` in /{src}/"));
                    }
                    // Aucun groupe nommé dans le pattern : Annex B
                    // retombe sur l'identité `\k` + `<name>` brut.
                    return Ok(("k".into(), i + 2));
                }
                if strict_k {
                    return Err(format!("malformed \\k<…> in regex /{src}/"));
                }
                return Ok(("k".into(), i + 2));
            }
            if strict_k {
                return Err(format!("invalid escape `\\k` in regex /{src}/"));
            }
            return Ok(("k".into(), i + 2));
        }
        if c == '0' {
            if !cs.get(i + 2).is_some_and(char::is_ascii_digit) {
                return Ok(("\\x{0}".into(), i + 2));
            }
            if self.unicode {
                return Err(format!("\\0 followed by digit in `u` regex /{src}/"));
            }
            // `\0` + chiffre : octale legacy Annex B (`\07`,
            // `\077`) — ou NUL + '8'/'9' littéral.
            let mut v = 0u32;
            let mut j = i + 2;
            let mut nd = 0;
            while nd < 2 {
                match cs.get(j).and_then(|d| d.to_digit(8)) {
                    Some(o) => {
                        v = v * 8 + o;
                        j += 1;
                        nd += 1;
                    }
                    None => break,
                }
            }
            return Ok((format!("\\x{{{v:x}}}"), j));
        }
        if c.is_ascii_digit() {
            return self.decimal_escape(cs, i, src);
        }
        if c == 'c' && cs.get(i + 2).is_some_and(char::is_ascii_alphabetic) {
            let v = u32::from(cs[i + 2]) % 32;
            return Ok((format!("\\x{{{v:x}}}"), i + 3));
        }
        if (c == 'p' || c == 'P') && self.unicode && cs.get(i + 2) == Some(&'{') {
            // `\p{Name}` / `\p{Name=Value}` — classe Unicode
            // passée telle quelle à fancy-regex.
            let rel_end = cs[i + 2..]
                .iter()
                .position(|&ch| ch == '}')
                .ok_or_else(|| format!("unterminated \\p{{ in `u` regex /{src}/"))?;
            let end = i + 2 + rel_end;
            let body: String = cs[i + 3..end].iter().collect();
            if body.is_empty()
                || !body
                    .chars()
                    .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '=')
            {
                return Err(format!("invalid \\p{{ name in `u` regex /{src}/"));
            }
            return Ok((cs[i..=end].iter().collect::<String>(), end + 1));
        }
        if c.is_ascii_alphanumeric() {
            let known = matches!(
                c,
                'd' | 'D'
                    | 's'
                    | 'S'
                    | 'w'
                    | 'W'
                    | 'b'
                    | 'B'
                    | 'f'
                    | 'n'
                    | 'r'
                    | 't'
                    | 'v'
                    | 'p'
                    | 'P'
            );
            if self.unicode && !known {
                return Err(format!("invalid escape `\\{c}` in `u` regex /{src}/"));
            }
            if !known {
                // Annex B sloppy : identity escape littéral —
                // `\c` non suivi d'une lettre garde son backslash.
                if c == 'c' {
                    return Ok(("\\\\c".into(), i + 2));
                }
                return Ok((c.to_string(), i + 2));
            }
            return Ok((format!("\\{c}"), i + 2));
        }
        if !self.unicode && !c.is_ascii() {
            return Ok((c.to_string(), i + 2));
        }
        Ok((format!("\\{c}"), i + 2))
    }

    /// `\N…` : backref décimale, avec les retombées Annex B quand
    /// le numéro excède les groupes du pattern (préfixe valide,
    /// octale legacy `\1`–`\7`, identité `\8`/`\9`).
    fn decimal_escape(&self, cs: &[char], i: usize, src: &str) -> Result<(String, usize), String> {
        let mut j = i + 1;
        let mut n = 0usize;
        while let Some(v) = cs.get(j).and_then(|d| d.to_digit(10)) {
            n = n.saturating_mul(10).saturating_add(v as usize);
            j += 1;
        }
        if n >= 1 && n <= self.n_groups {
            if self.closed.contains(&n) {
                if self.need_synth {
                    // Le backend refuse `\N` en présence de
                    // groupes nommés — réémet `\k<émis>` (JS-C269).
                    if let Some(nm) = self.emitted.get(n - 1).filter(|s| !s.is_empty()) {
                        return Ok((format!("\\k<{nm}>"), j));
                    }
                }
                return Ok((format!("\\{n}"), j));
            }
            // Backref « vers l'avant » : le groupe n'a pas encore
            // fermé — il ne peut pas avoir capturé → la référence
            // matche la chaîne vide (spec 21.2.2.9, même traitement
            // que les `\k<…>` prédéclarées). `/(\3)(\1)(a)/` sur
            // « cat » doit matcher « a » (Acid3 t90).
            return Ok(("(?:)".into(), j));
        }
        if self.unicode {
            return Err(format!("invalid backref \\{n} in /{src}/"));
        }
        // Annex B : le nombre **entier** tranche — `n` au-delà
        // des groupes → retombe en octale/identité ci-dessous
        // (`\141` = `\x{61}` même avec des groupes — le « plus
        // long préfixe » émettait `\1`+`41`, matchait la
        // chaîne « 41 » au lieu de 'a' : §B-56 bras 3).
        let d0 = cs[i + 1];
        if matches!(d0, '8' | '9') {
            return Ok((d0.to_string(), i + 2));
        }
        let mut v = 0u32;
        let mut k = i + 1;
        let mut nd = 0;
        while nd < 3 {
            match cs.get(k).and_then(|d| d.to_digit(8)) {
                Some(o) => {
                    v = v * 8 + o;
                    k += 1;
                    nd += 1;
                }
                None => break,
            }
        }
        Ok((format!("\\x{{{v:x}}}"), k))
    }
}

/// `\uD8xx\uDCxx` à `cs[i]` → point de code fusionné (mode `u`
/// suivant) — paire de surrogates complète uniquement.
fn surrogate_pair(cs: &[char], i: usize) -> Option<(char, usize)> {
    // cs[i] == '\\', cs[i + 1] == 'u'
    let hi = hex4(cs, i + 2)?;
    if !(0xd800..0xdc00).contains(&hi) {
        return None;
    }
    if cs.get(i + 6) != Some(&'\\') || cs.get(i + 7) != Some(&'u') {
        return None;
    }
    let lo = hex4(cs, i + 8)?;
    if !(0xdc00..0xe000).contains(&lo) {
        return None;
    }
    let cp = 0x1_0000 + ((hi - 0xd800) << 10) + (lo - 0xdc00);
    char::from_u32(cp).map(|c| (c, i + 12))
}

/// 4 chiffres hexadécimaux à partir de `at`.
fn hex4(cs: &[char], at: usize) -> Option<u32> {
    let mut v = 0u32;
    for k in 0..4 {
        v = v * 16 + cs.get(at + k)?.to_digit(16)?;
    }
    Some(v)
}

/// Backtrack step bound — a hostile pattern must not freeze the
/// engine (the web is hostile by default).
const BACKTRACK_LIMIT: usize = 100_000;

/// Borne NFA des sous-regex déléguées au backend `regex` —
/// `{n,m}` sur une grosse classe Unicode (`\w`, `[\s\S]`)
/// déroule ~`m` copies d'états : le défaut 10 Mio tuait des
/// patterns valides du web réel (§B-56 — frandroid
/// `/^[\w:-]{32,512}$/` ≈ 30 Mio d'états). 64 Mio couvre
/// `{…,≤~1024}` sur `\w` ; au-delà `SyntaxError`
/// (dégradation bornée, pas de gel).
const DELEGATE_SIZE_LIMIT: usize = 64 << 20;

/// Trim the regex engine's multi-line error down to its first line.
fn short_regex_error(e: &fancy_regex::Error) -> String {
    e.to_string()
        .lines()
        .next()
        .unwrap_or("invalid pattern")
        .to_string()
}

/// Char index → byte offset in `hay` (`len` past the end).
pub(crate) fn char_to_byte(hay: &str, char_pos: usize) -> usize {
    hay.char_indices()
        .nth(char_pos)
        .map_or(hay.len(), |(b, _)| b)
}

fn byte_to_char(hay: &str, byte: usize) -> usize {
    hay[..byte.min(hay.len())].chars().count()
}

#[cfg(test)]
mod tests {
    use super::translate_pattern;

    fn tr(src: &str) -> String {
        translate_pattern(src, false, false, false, false)
            .unwrap()
            .0
    }

    /// `-` littéral en classe (tête/queue) : émis `\x{2d}` et
    /// compté dans `class_emitted` — sinon la classe dégénérait
    /// en `(?!)` (`/[-]/` ne matchait rien, `^[-]?` rejeté par le
    /// backend — ecma-international contact-form-7).
    #[test]
    fn literal_dash_in_class() {
        assert_eq!(tr(r"[-]"), "[\\x{2d}]");
        assert_eq!(tr(r"[-a]"), "[\\x{2d}a]");
        assert_eq!(tr(r"[a-]"), "[a\\x{2d}]");
        assert_eq!(tr(r"[a-b-c]"), "[a-b\\x{2d}c]");
        let probe = |src: &str, hay: &str| super::JsRegExp::compile(src, "").unwrap().exec(hay, 0);
        assert!(probe(r"[-]", "-").is_some());
        assert!(probe(r"^[-]?[0-9]+(?:[eE][+-]?[0-9]+)?$", "-3").is_some());
        assert!(probe(r"^[-]?[0-9]+(?:[eE][+-]?[0-9]+)?$", "12e+4").is_some());
    }

    /// Atome hors classe = paire de surrogates `\uD8xx\uDCxx` :
    /// fusionnée en point astral (le `frag` brut `\x{d8xx}` était
    /// rejeté « Invalid codepoint » — regex emoji de gitlab.com).
    /// En `u` `escape()` fusionnait déjà ; sloppy est aligné.
    #[test]
    fn surrogate_pair_atom_outside_class() {
        // JS-C622 : non-`u` = domaine units éclatés — la paire
        // devient deux sentinelles groupées en un atome.
        assert_eq!(tr(r"\uD83D\uDCA5"), "(?:\\x{f003d}\\x{f04a5})");
        assert_eq!(
            tr(r"\uD83C\uDFFB-\uD83C\uDFFF"),
            "(?:\\x{f003c}\\x{f07fb})-(?:\\x{f003c}\\x{f07ff})"
        );
        // Isolé (pas de paire) → sentinelle + astrals du lead.
        assert_eq!(tr(r"\uD83Dx"), "[\\x{f003d}\\x{1f400}-\\x{1f7ff}]x");
        // `exec` opère dans le domaine encodé — non-`u` = units
        // éclatés (JS-C622) : le haystack astral y est converti
        // comme `enc_hay` le fait à l'exécution.
        let probe = |src: &str, hay: &str| {
            let units: Vec<u16> = hay.encode_utf16().collect();
            let enc = super::re_encode_split(&units);
            super::JsRegExp::compile(src, "").unwrap().exec(&enc, 0)
        };
        assert!(probe(r"\uD83D\uDCA5", "\u{1f4a5}").is_some());
        assert!(probe(r"\uD83C[\uDFFB-\uDFFF]", "\u{1f3fd}").is_some());
    }

    /// §B-56 bras 3 (airfrance vol 101 — jscrambler) —
    /// `\N` chiffre EN CLASSE = octale/identité Annex B,
    /// jamais backref : `escape` rendait `\1(?:00)` pour
    /// `[\r\100]` → « Invalid character class ». Hors
    /// classe, le nombre **entier** tranche : `\141` >
    /// groupes → octale `\x{61}` (le « plus long préfixe »
    /// émettait `\1`+`41` — matchait « 41 » au lieu de 'a').
    #[test]
    fn class_decimal_is_octal_not_backref() {
        // Format `\x{…}` uniforme avec `escape()`.
        assert_eq!(tr(r"[\100]"), "[\\x{40}]");
        assert_eq!(tr(r"[\141]"), "[\\x{61}]");
        // `\18` = `\x{1}` puis `8` littéral (non-octal stoppe).
        assert_eq!(tr(r"[\18]"), "[\\x{1}8]");
        // Identité `\8` puis `9` littéral.
        assert_eq!(tr(r"[\89]"), "[\\x{38}9]");
        // Hors classe avec groupes : entier > groupes → octale.
        assert_eq!(tr(r"(a)\141"), "(a)\\x{61}");
        // Verbatim jscrambler airfrance — compile entier.
        super::JsRegExp::compile(
            r"(?:^|\141\164[\f\u1680-\u2000\u200a\t\u2029 \ufeff\r\u202f\u00a0\u205f\v\u2028\n\u3000]{1,})(?:([^\n\r\u2028\u2029]{0,}?)[\v\u3000\u00a0\f\u200a\ufeff\u202f\u2028 \u1680-\u2000\n\u2029\u205f\t\r\100]\x28){0,1}([^\u2028\r\n\u2029]{1,}?)\u003a([0-9]{1,})\u003a([0-9]{1,})\051{0,1}$",
            "",
        )
        .unwrap();
    }

    /// Bornes de plage : les échappements sont décodés — bug
    /// mesuré : `\x00` lu comme 'x' (0x78) → "empty class range"
    /// sur `/[\x00- ]/` (pattern réel d'un bundle Google).
    #[test]
    fn class_escape_bounds_decode() {
        assert_eq!(tr(r"[\x00- ]"), "[\\x{0}-\\x{20}]");
        assert_eq!(tr(r"[\x20-\x30]"), "[\\x{20}-\\x{30}]");
        assert_eq!(tr(r"[\u2020-\u2030]"), "[\\x{2020}-\\x{2030}]");
        // JS-C622 : `[\u{1F600}-\u{1F64F}]` non-`u` = range units
        // DE00–D83D désordonnée → SyntaxError (comportement V8).
        assert!(super::JsRegExp::compile(r"[\u{1F600}-\u{1F64F}]", "").is_err());
        assert_eq!(tr(r"[\0-\x1f]"), "[\\x{0}-\\x{1f}]");
        assert_eq!(tr(r"[\cA-\cZ]"), "[\\x{1}-\\x{1a}]");
        assert_eq!(tr(r"[\101-\103]"), "[\\x{41}-\\x{43}]");
        assert_eq!(
            tr(r"[\x00- \x22\x27-\x29\x3c\x3e\\\x7b\x7d\x7f\x85\xa0\u2028\u2029\uff01\uff03\uff04\uff06-\uff0c\uff0f\uff1a\uff1b\uff1d\uff1f\uff20\uff3b\uff3d]"),
            "[\\x{0}-\\x{20}\\x{22}\\x{27}-\\x{29}\\x{3c}\\x{3e}\\\\\\x{7b}\\x{7d}\\x{7f}\\x{85}\\x{a0}\\x{2028}\\x{2029}\\x{ff01}\\x{ff03}\\x{ff04}\\x{ff06}-\\x{ff0c}\\x{ff0f}\\x{ff1a}\\x{ff1b}\\x{ff1d}\\x{ff1f}\\x{ff20}\\x{ff3b}\\x{ff3d}]"
        );
    }

    /// Surrogates : domaine sentinelle — l'atome isolé est un
    /// char sentinelle (+ astrals du lead en non-`u`), les plages
    /// émettent l'union scalaire + sentinelles + astrals leads.
    #[test]
    fn surrogate_bounds_degrade() {
        // Plage entièrement surrogate = sentinelles + « tout
        // atome astral » en sémantique UTF-16 (minifieurs).
        assert_eq!(
            tr(r"[\uD800-\uDFFF]"),
            "[\\x{f0000}-\\x{f07ff}\\x{10000}-\\x{10ffff}]"
        );
        assert_eq!(
            tr(r"[^\uD800-\uDBFF]"),
            "[^\\x{f0000}-\\x{f03ff}\\x{10000}-\\x{10ffff}]"
        );
        assert_eq!(
            tr(r"[\uD800-\uE000]"),
            "[\\x{e000}-\\x{e000}\\x{f0000}-\\x{f07ff}\\x{10000}-\\x{10ffff}]"
        );
        assert_eq!(
            tr(r"[a-\uDFFF]"),
            "[a-\\x{d7ff}\\x{f0000}-\\x{f07ff}\\x{10000}-\\x{10ffff}]"
        );
        assert_eq!(tr(r"\uD800"), "[\\x{f0000}\\x{10000}-\\x{103ff}]");
        // `lo > hi` brut reste une erreur (surrogate > ASCII).
        assert!(translate_pattern(r"[\uD800-z]", false, false, false, false).is_err());
        // Le pattern réel du bundle Google compile et ne matche
        // rien (nos strings n'ont pas de surrogate orphelin).
        let t = tr(r"(?:[^\uD800-\uDBFF]|^)[\uDC00-\uDFFF]|[\uD800-\uDBFF](?![\uDC00-\uDFFF])");
        assert!(!t.contains("d800") && !t.contains("dc00") && !t.contains("dbff"));
    }

    /// Le résultat traduit matche ce qu'il doit matcher.
    #[test]
    fn class_escapes_exec() {
        let probe = |src: &str, hay: &str| {
            let re = super::JsRegExp::compile(src, "").unwrap();
            re.exec(hay, 0).is_some()
        };
        assert!(probe(r"[\x00- ]", " "));
        assert!(!probe(r"[\x00- ]", "a"));
        assert!(probe(r"[\x20-\x30]", "%"));
        assert!(probe(r"[\u2020-\u2030]", "\u{2025}"));
        // Range astrale non-`u` = units désordonnées → rejetée.
        assert!(super::JsRegExp::compile(r"[\u{1F600}-\u{1F64F}]", "").is_err());
        assert!(probe(r"[\0-\x1f]", "\x07"));
        assert!(probe(r"[\cA-\cZ]", "\x0b"));
        assert!(probe(r"[\101-\103]", "B"));
        assert!(!probe(r"[\uD800-\uDFFF]", "a"));
        assert!(probe(r"[^\uD800-\uDBFF]", "a"));
        assert!(probe(r"[\uD800-\uE000]", "\u{e000}"));
        assert!(probe(r"[a-\uDFFF]", "m"));
        assert!(!probe(r"\uD800", "a"));
        assert!(!probe(
            r"(?:[^\uD800-\uDBFF]|^)[\uDC00-\uDFFF]|[\uD800-\uDBFF](?![\uDC00-\uDFFF])",
            "abc"
        ));
    }
}

#[cfg(test)]
mod dbg_vol10 {
    #[test]
    fn yt_regex() {
        let pat = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/data/vol10_yt_regex.txt"
        ))
        .unwrap();
        let t = super::translate_pattern(&pat, false, false, false, false)
            .unwrap()
            .0;
        eprintln!("TAIL>>{}", &t[1850.min(t.len())..]);
        assert!(!t.contains("(?!)"), "dead branch left: {t}");
        assert!(fancy_regex::Regex::new(&t).is_ok());
        // Plage astrale `\uD800\uDD00-\uDD02` = U+10300–U+10302 :
        // la classe multi-items doit la couvrir.
        // `\uD800\uDD00` = U+10100, `\uD800\uDF9F` = U+1039F.
        let mini = super::translate_pattern("\\uD800[\\uDD00-\\uDD02]", false, false, false, false)
            .unwrap()
            .0;
        // JS-C622 non-`u` : units éclatés — lead sentinelle +
        // classe de sentinelles trail.
        assert_eq!(mini, "\\x{f0000}[\\x{f0500}-\\x{f0502}]");
        let re = super::JsRegExp::compile(&pat, "").unwrap();
        let enc = |s: &str| super::re_encode_split(&s.encode_utf16().collect::<Vec<_>>());
        assert!(re.exec(&enc("\u{10101}"), 0).is_some());
        assert!(re.exec(&enc("\u{1039f}"), 0).is_some());
    }
}
