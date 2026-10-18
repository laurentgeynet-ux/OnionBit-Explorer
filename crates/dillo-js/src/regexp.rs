// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! `RegExp` semantics for `dillo-js`, built on the pure-Rust `regex`
//! crate.
//!
//! Coverage: literals, `g i m s u y` flags, `exec`/`test` with
//! `lastIndex`, `String.match`/`replace`/`split`/`search`.
//!
//! Deliberate limitation: the `regex` crate has no look-around or
//! backreferences — JS patterns using `(?=)`, `(?!`, `(?<=`,
//! `(?<!` or `\1` fail to compile and surface as `SyntaxError`
//! (script-visible, catchable). Everything the jQuery era commonly
//! uses — classes, quantifiers, anchors, alternation, captures,
//! `(?:…)` — works.

/// A compiled JS regular expression plus its per-object flags.
#[allow(clippy::struct_excessive_bools)] // flags are flags.
pub struct JsRegExp {
    /// Compiled pattern (JS `i`/`m`/`s` folded into inline flags).
    re: regex::Regex,
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
        for f in flags.chars() {
            match f {
                'g' if !global => global = true,
                'i' if !ignore_case => ignore_case = true,
                'm' if !multiline => multiline = true,
                's' if !dot_all => dot_all = true,
                'y' if !sticky => sticky = true,
                'u' if !unicode => unicode = true,
                'd' | 'v' => {} // accepted, no observable effect here
                _ => return Err(format!("invalid regex flag `{f}`")),
            }
        }
        let mut pat = String::new();
        if ignore_case {
            pat.push_str("(?i)");
        }
        if multiline {
            pat.push_str("(?m)");
        }
        if dot_all {
            pat.push_str("(?s)");
        }
        pat.push_str(&translate_pattern(source));
        let re = regex::Regex::new(&pat)
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
        })
    }

    /// `/src/flags` rendering (`toString`).
    pub fn flag_string(&self) -> String {
        let mut f = String::new();
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
        let caps = self.re.captures_at(hay, byte_start)?;
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
            match self.re.captures_at(hay, pos) {
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

    fn build_match(&self, hay: &str, caps: &regex::Captures) -> ExecMatch {
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
        for name in self.re.capture_names().flatten() {
            if let Some(g) = caps.name(name) {
                named.push((name.to_string(), g.as_str().to_string()));
            }
        }
        ExecMatch {
            index,
            end,
            start_b,
            end_b,
            text,
            groups,
            named,
        }
    }
}

/// `(?<name>…)` → `(?P<name>…)`; everything else passes through
/// (regex-crate syntax is a superset of what we need).
fn translate_pattern(src: &str) -> String {
    let mut out = String::with_capacity(src.len() + 4);
    let cs: Vec<char> = src.chars().collect();
    let mut i = 0;
    let mut in_class = false;
    while i < cs.len() {
        match cs[i] {
            '\\' => {
                out.push('\\');
                if let Some(&c) = cs.get(i + 1) {
                    out.push(c);
                    i += 2;
                } else {
                    i += 1;
                }
                continue;
            }
            '[' => in_class = true,
            ']' => in_class = false,
            '(' if !in_class
                && cs.get(i + 1) == Some(&'?')
                && cs.get(i + 2) == Some(&'<')
                && !matches!(cs.get(i + 3), Some('=' | '!')) =>
            {
                out.push_str("(?P<");
                i += 3;
                continue;
            }
            _ => {}
        }
        out.push(cs[i]);
        i += 1;
    }
    out
}

/// Trim the `regex` crate's multi-line error down to its first line.
fn short_regex_error(e: &regex::Error) -> String {
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
