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
    /// `d` — `exec` results carry `indices` (start/end per group).
    pub has_indices: bool,
}

/// One `exec` result, in *char* units (the engine's strings are
/// char-indexed — JS UTF-16 code units are approximated by chars).
pub struct ExecMatch {
    /// Char index of the whole match.
    pub index: usize,
    /// Char offset just past the whole match (new `lastIndex`).
    pub end: usize,
    /// Byte index of the whole match (for splicing).
    pub start_b: usize,
    /// Byte offset just past the whole match.
    pub end_b: usize,
    /// Whole match text.
    pub text: String,
    /// `groups[0]` == `text`; `groups[i]` is capture `i`.
    pub groups: Vec<Option<String>>,
    /// `(?<name>…)` captures.
    pub named: Vec<(String, String)>,
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
                'v' => {} // accepted, unicodeSets not implemented
                _ => return Err(format!("invalid regex flag `{f}`")),
            }
        }
        let mut pat = String::new();
        // `i` : le backend fait du folding Unicode complet — exact
        // en mode `u`, trop large en non-`u` où JS canonise par
        // `ToUppercase` simple (`/\u{212a}/i` ne doit pas matcher
        // 'k'). Non-`u` → folding manuel dans `translate_pattern`.
        if ignore_case && unicode {
            pat.push_str("(?i)");
        }
        if multiline {
            pat.push_str("(?m)");
        }
        if dot_all {
            pat.push_str("(?s)");
        }
        pat.push_str(&translate_pattern(
            source,
            unicode,
            ignore_case && !unicode,
        )?);
        let re = fancy_regex::RegexBuilder::new(&pat)
            .backtrack_limit(BACKTRACK_LIMIT)
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
            has_indices,
        })
    }

    /// `/src/flags` rendering (`toString`) — ordre canonique
    /// `dgimsuy` (spec `RegExp.prototype.flags`).
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
        if self.sticky {
            f.push('y');
        }
        f
    }

    /// First match at or after `start` (char units). With `sticky`,
    /// the match must begin exactly at `start`.
    pub fn exec(&self, hay: &str, start: usize) -> Option<ExecMatch> {
        let byte_start = char_to_byte(hay, start);
        let caps = self.re.captures_from_pos(hay, byte_start).ok()??;
        let m = caps.get(0)?;
        if self.sticky && byte_to_char(hay, m.start()) != start {
            return None;
        }
        Some(self.build_match(hay, &caps))
    }

    /// `exec` driving `lastIndex`: `g`/`y` read and advance it —
    /// the object-side bookkeeping lives in builtins.
    pub fn exec_from(&self, hay: &str, last_index: usize) -> Option<(ExecMatch, usize)> {
        let m = self.exec(hay, last_index)?;
        let new_last = if m.end > last_index { m.end } else { m.end + 1 };
        Some((m, new_last))
    }

    /// All matches of a `g` regex from `start`, empty matches bumped
    /// one char forward (spec: `RegExpExec` on empty match advances).
    pub fn exec_all(&self, hay: &str, start: usize) -> Vec<ExecMatch> {
        let mut out = Vec::new();
        let mut pos = start;
        let hay_len = hay.chars().count();
        while pos <= hay_len {
            match self.exec(hay, pos) {
                None => break,
                Some(m) => {
                    let next = if m.end == m.index { m.end + 1 } else { m.end };
                    pos = next;
                    out.push(m);
                }
            }
        }
        out
    }

    /// `str.split(re)` pieces + captured-group insertions (spec puts
    /// capture groups between the literal pieces).
    pub fn split<'h>(&self, hay: &'h str) -> Vec<&'h str> {
        let mut out = Vec::new();
        let mut last = 0usize; // byte offset
        let hay_len = hay.len();
        let mut pos = 0usize; // byte offset
        while pos <= hay_len {
            match self.re.captures_from_pos(hay, pos).ok().flatten() {
                None => break,
                Some(caps) => {
                    let Some(m) = caps.get(0) else { break };
                    if self.sticky && m.start() != pos {
                        break;
                    }
                    out.push(&hay[last..m.start()]);
                    for i in 1..caps.len() {
                        if let Some(g) = caps.get(i) {
                            out.push(&hay[g.start()..g.end()]);
                        }
                    }
                    last = m.end();
                    pos = if m.end() == m.start() {
                        next_boundary(hay, m.end())
                    } else {
                        m.end()
                    };
                }
            }
        }
        out.push(&hay[last..]);
        out
    }

    fn build_match(&self, hay: &str, caps: &fancy_regex::Captures) -> ExecMatch {
        let m = caps.get(0);
        let (index, end, start_b, end_b, text) = match m {
            Some(m) => (
                byte_to_char(hay, m.start()),
                byte_to_char(hay, m.end()),
                m.start(),
                m.end(),
                m.as_str().to_string(),
            ),
            None => (0, 0, 0, 0, String::new()),
        };
        let mut groups = Vec::with_capacity(caps.len());
        groups.push(Some(text.clone()));
        for i in 1..caps.len() {
            groups.push(caps.get(i).map(|g| g.as_str().to_string()));
        }
        let mut named = Vec::new();
        let mut named_indices = Vec::new();
        for name in self.re.capture_names().flatten() {
            if let Some(g) = caps.name(name) {
                named.push((name.to_string(), g.as_str().to_string()));
                named_indices.push((
                    name.to_string(),
                    Some((byte_to_char(hay, g.start()), byte_to_char(hay, g.end()))),
                ));
            } else {
                named_indices.push((name.to_string(), None));
            }
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
            start_b,
            end_b,
            text,
            groups,
            named,
            group_indices,
            named_indices,
        }
    }
}

/// `(?<name>…)` → `(?P<name>…)`; everything else passes through
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
fn translate_pattern(src: &str, unicode: bool, fold: bool) -> Result<String, String> {
    let cs: Vec<char> = src.chars().collect();
    let (all_names, n_groups) = prescan(&cs);
    let mut t = Trans {
        unicode,
        fold,
        n_groups,
        names: std::collections::BTreeSet::new(),
        all_names,
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
            let (frag, next) = t.escape(&cs, i, src)?;
            if in_class {
                // Surrogate : nos strings UTF-8 n'en contiennent
                // jamais — l'atome ne peut matcher → omis (il reste
                // compté comme borne de range éventuelle).
                let atom = class_atom_of(&cs, i);
                if let ClassAtom::Char(cp) = atom {
                    if is_surrogate(cp) {
                        class_prev = Some(atom);
                        class_prev_bound = class_dash;
                        class_dash = false;
                        i = next;
                        continue;
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
                // Hors classe : un atome surrogate ne peut matcher
                // — `(?!)` (jamais) plutôt qu'un codepoint invalide
                // pour le backend.
                if let ClassAtom::Char(cp) = class_atom_of(&cs, i) {
                    if is_surrogate(cp) {
                        out.push_str("(?!)");
                        last_atom = true;
                        i = next;
                        continue;
                    }
                }
                emit_escape(&mut out, &frag, t.fold);
                last_atom = true;
            }
            i = next;
            continue;
        }
        match c {
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
                    // Sloppy : `-` littéral devant une class-escape.
                    out.push_str("\\-");
                    class_emitted = true;
                    class_prev = Some(ClassAtom::Char(u32::from('-')));
                    class_prev_bound = false;
                } else if is_range {
                    if let (Some(ClassAtom::Char(lo)), ClassAtom::Char(hi)) = (class_prev, next) {
                        if lo > hi {
                            return Err(format!("empty class range in /{src}/"));
                        }
                        // Bornes surrogate : jamais présentes dans
                        // nos strings UTF-8 — la plage est clampée
                        // (`lo`→`\x{e000}`, `hi`→`\x{d7ff}`) ; une
                        // plage entièrement surrogate n'émet rien.
                        let lo_c = if is_surrogate(lo) { 0xe000 } else { lo };
                        let hi_c = if is_surrogate(hi) { 0xd7ff } else { hi };
                        // La borne haute est toujours consommée ici
                        // (fold ou non) — l'émission décalée au tour
                        // suivant empêcherait le clamp.
                        let (hfrag, hnext) = if cs[i + 1] == '\\' {
                            t.escape(&cs, i + 1, src)?
                        } else {
                            (cs[i + 1].to_string(), i + 2)
                        };
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
                        class_prev = Some(ClassAtom::Char(hi));
                        class_prev_bound = true;
                        class_dash = false;
                        i = hnext;
                        continue;
                    }
                    out.push('-');
                    class_dash = true;
                } else {
                    out.push_str("\\-");
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
                out.push('(');
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
                out.push('.');
            }
            _ => {
                if in_class {
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
    Ok(out)
}

/// État partagé du traducteur : flags, tables de groupes.
struct Trans {
    unicode: bool,
    /// `i` en mode non-`u` : folding `ToUppercase` simple émis à la
    /// main (le `i` du backend est un folding Unicode complet).
    fold: bool,
    /// Nombre de groupes capturants du pattern (validation `\N`).
    n_groups: usize,
    /// Noms déclarés *avant* la position courante.
    names: std::collections::BTreeSet<String>,
    /// Tous les noms du pattern (backref avant déclaration).
    all_names: std::collections::BTreeSet<String>,
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

/// Premier passage : tous les noms de groupes capturants + leur
/// nombre (les `\k<name>`/`\N` sont validés contre cette table —
/// une backref nommée peut précéder sa déclaration en JS).
fn prescan(cs: &[char]) -> (std::collections::BTreeSet<String>, usize) {
    let mut all = std::collections::BTreeSet::new();
    let mut n = 0usize;
    let mut in_class = false;
    let mut i = 0;
    while i < cs.len() {
        match cs[i] {
            '\\' => i += 1,
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
    (all, n)
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
            out.push_str("(?P<");
            out.push_str(&name);
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
                        return Ok((format!("\\k<{name}>"), j + 1));
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
        // Annex B : plus long préfixe désignant un groupe (`\12`
        // avec 3 groupes → `\1` + '2'), sinon octale/identité.
        let mut k = i + 1;
        let mut v = 0usize;
        let mut best = None;
        while k < j {
            v = v * 10 + (u32::from(cs[k]) - u32::from('0')) as usize;
            k += 1;
            if (1..=self.n_groups).contains(&v) {
                best = Some(k);
            }
        }
        if let Some(k) = best {
            let prefix: String = cs[i + 1..k].iter().collect();
            let rest: String = cs[k..j].iter().collect();
            // `(?:rest)` après la backref : `\1` + `23` écrit `\123`
            // serait relu comme la backref 123. Préfixe « vers
            // l'avant » → match vide, comme la branche au-dessus.
            let pn: usize = prefix.parse().unwrap_or(0);
            let head = if self.closed.contains(&pn) {
                format!("\\{prefix}")
            } else {
                "(?:)".to_string()
            };
            return Ok((format!("{head}(?:{rest})"), j));
        }
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

/// Trim the regex engine's multi-line error down to its first line.
fn short_regex_error(e: &fancy_regex::Error) -> String {
    e.to_string()
        .lines()
        .next()
        .unwrap_or("invalid pattern")
        .to_string()
}

fn char_to_byte(hay: &str, char_pos: usize) -> usize {
    hay.char_indices()
        .nth(char_pos)
        .map_or(hay.len(), |(b, _)| b)
}

fn byte_to_char(hay: &str, byte: usize) -> usize {
    hay[..byte.min(hay.len())].chars().count()
}

fn next_boundary(hay: &str, byte: usize) -> usize {
    let mut b = byte + 1;
    while b < hay.len() && !hay.is_char_boundary(b) {
        b += 1;
    }
    b
}

#[cfg(test)]
mod tests {
    use super::translate_pattern;

    fn tr(src: &str) -> String {
        translate_pattern(src, false, false).unwrap()
    }

    /// Bornes de plage : les échappements sont décodés — bug
    /// mesuré : `\x00` lu comme 'x' (0x78) → "empty class range"
    /// sur `/[\x00- ]/` (pattern réel d'un bundle Google).
    #[test]
    fn class_escape_bounds_decode() {
        assert_eq!(tr(r"[\x00- ]"), "[\\x{0}-\\x{20}]");
        assert_eq!(tr(r"[\x20-\x30]"), "[\\x{20}-\\x{30}]");
        assert_eq!(tr(r"[\u2020-\u2030]"), "[\\x{2020}-\\x{2030}]");
        assert_eq!(tr(r"[\u{1F600}-\u{1F64F}]"), "[\\x{1f600}-\\x{1f64f}]");
        assert_eq!(tr(r"[\0-\x1f]"), "[\\x{0}-\\x{1f}]");
        assert_eq!(tr(r"[\cA-\cZ]"), "[\\x{1}-\\x{1a}]");
        assert_eq!(tr(r"[\101-\103]"), "[\\x{41}-\\x{43}]");
        assert_eq!(
            tr(r"[\x00- \x22\x27-\x29\x3c\x3e\\\x7b\x7d\x7f\x85\xa0\u2028\u2029\uff01\uff03\uff04\uff06-\uff0c\uff0f\uff1a\uff1b\uff1d\uff1f\uff20\uff3b\uff3d]"),
            "[\\x{0}-\\x{20}\\x{22}\\x{27}-\\x{29}\\x{3c}\\x{3e}\\\\\\x{7b}\\x{7d}\\x{7f}\\x{85}\\x{a0}\\x{2028}\\x{2029}\\x{ff01}\\x{ff03}\\x{ff04}\\x{ff06}-\\x{ff0c}\\x{ff0f}\\x{ff1a}\\x{ff1b}\\x{ff1d}\\x{ff1f}\\x{ff20}\\x{ff3b}\\x{ff3d}]"
        );
    }

    /// Surrogates : jamais présents dans nos strings UTF-8 — atome
    /// omis/(?!), plages clampées, classe vidée → `(?!)`/`[\s\S]`.
    #[test]
    fn surrogate_bounds_degrade() {
        assert_eq!(tr(r"[\uD800-\uDFFF]"), "(?!)");
        assert_eq!(tr(r"[^\uD800-\uDBFF]"), "[\\s\\S]");
        assert_eq!(tr(r"[\uD800-\uE000]"), "[\\x{e000}-\\x{e000}]");
        assert_eq!(tr(r"[a-\uDFFF]"), "[a-\\x{d7ff}]");
        assert_eq!(tr(r"\uD800"), "(?!)");
        // `lo > hi` brut reste une erreur (surrogate > ASCII).
        assert!(translate_pattern(r"[\uD800-z]", false, false).is_err());
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
        assert!(probe(r"[\u{1F600}-\u{1F64F}]", "\u{1f610}"));
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
