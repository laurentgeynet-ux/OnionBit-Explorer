// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Tokenizer CSS (spec CSS Syntax Level 3) — une passe sur
//! l'entrée déjà décodée. Tolérant : `bad-string`/`bad-url` sont
//! émis au lieu de paniquer ; les échappements sont résolus.

use crate::token::Token;

/// Nombre max de tokens émis (borne contenu hostile — ADR-0011).
const MAX_TOKENS: usize = 256 * 1024;

/// Tokenize tout le flux — retourne sans `Eof` final (le parser
/// traite la fin par épuisement).
#[must_use]
pub fn tokenize(input: &str) -> Vec<Token> {
    let mut t = Tokenizer::new(input);
    let mut out = Vec::new();
    loop {
        let tok = t.next_token();
        match tok {
            Token::Eof => break,
            _ if out.len() >= MAX_TOKENS => break,
            _ => out.push(tok),
        }
    }
    out
}

struct Tokenizer<'a> {
    chars: std::str::Chars<'a>,
    /// Buffer d'avance — la TÊTE (`front`) est le prochain char.
    /// `peek` pousse en `back`, un « unread » pousse en `front`.
    peeked: std::collections::VecDeque<char>,
}

fn is_name_start(c: char) -> bool {
    c.is_ascii_alphabetic() || c == '_' || !c.is_ascii()
}

fn is_name_char(c: char) -> bool {
    is_name_start(c) || c.is_ascii_digit() || c == '-'
}

fn is_hex(c: char) -> bool {
    c.is_ascii_hexdigit()
}

fn is_ws(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\n' | '\r' | '\x0C')
}

impl<'a> Tokenizer<'a> {
    fn new(input: &'a str) -> Self {
        Self {
            chars: input.chars(),
            peeked: std::collections::VecDeque::new(),
        }
    }

    fn next_char(&mut self) -> Option<char> {
        if let Some(c) = self.peeked.pop_front() {
            Some(c)
        } else {
            self.chars.next()
        }
    }

    fn peek(&mut self, n: usize) -> Option<char> {
        while self.peeked.len() <= n {
            let c = self.chars.next()?;
            self.peeked.push_back(c);
        }
        self.peeked.get(n).copied()
    }

    /// Remet un char déjà consommé en tête (« unread »).
    fn unread(&mut self, c: char) {
        self.peeked.push_front(c);
    }

    /// Deux caractères commencent-ils un ident ? (`-x`, `--`, `-\_`,
    /// `x`, `\` échappé).
    fn starts_ident(&mut self) -> bool {
        match self.peek(0) {
            Some('-') => match self.peek(1) {
                Some(c) if is_name_start(c) || c == '-' => true,
                Some('\\') => self.valid_escape(1),
                _ => false,
            },
            Some(c) if is_name_start(c) => true,
            Some('\\') => self.valid_escape(0),
            _ => false,
        }
    }

    /// `\` suivi d'autre chose qu'un newline = échappement valide.
    fn valid_escape(&mut self, offset: usize) -> bool {
        self.peek(offset) == Some('\\')
            && !matches!(self.peek(offset + 1), Some('\n' | '\r' | '\x0C') | None)
    }

    fn starts_number(&mut self) -> bool {
        match self.peek(0) {
            Some('+' | '-') => match self.peek(1) {
                Some(c) if c.is_ascii_digit() => true,
                Some('.') => self.peek(2).is_some_and(|c| c.is_ascii_digit()),
                _ => false,
            },
            Some('.') => self.peek(1).is_some_and(|c| c.is_ascii_digit()),
            Some(c) => c.is_ascii_digit(),
            None => false,
        }
    }

    /// Lit un nom (ident) avec échappements.
    fn consume_name(&mut self) -> String {
        let mut out = String::new();
        while let Some(c) = self.next_char() {
            if is_name_char(c) {
                out.push(c);
            } else if c == '\\' {
                out.push(self.consume_escape());
            } else {
                self.unread(c);
                break;
            }
        }
        out
    }

    /// `\XXXXXX` hex ou `\c` littéral — `U+FFFD` sur nul/out-range.
    fn consume_escape(&mut self) -> char {
        match self.next_char() {
            Some(c) if is_hex(c) => {
                let mut value = c.to_digit(16).unwrap_or(0);
                for _ in 0..5 {
                    match self.peek(0) {
                        Some(h) if is_hex(h) => {
                            value = value * 16 + h.to_digit(16).unwrap_or(0);
                            self.next_char();
                        }
                        _ => break,
                    }
                }
                // Un whitespace après l'hexa est consommé.
                if self.peek(0).is_some_and(is_ws) {
                    self.next_char();
                }
                char::from_u32(value)
                    .filter(|&c| c != '\0')
                    .unwrap_or('\u{FFFD}')
            }
            Some('\n' | '\r' | '\x0C') | None => '\u{FFFD}',
            Some(c) => c,
        }
    }

    /// Lit un nombre : signe, décimales, exposant.
    fn consume_number(&mut self) -> (String, f64) {
        let mut repr = String::new();
        if let Some(c @ ('+' | '-')) = self.peek(0) {
            repr.push(c);
            self.next_char();
        }
        while self.peek(0).is_some_and(|c| c.is_ascii_digit()) {
            repr.push(self.next_char().unwrap_or_default());
        }
        if self.peek(0) == Some('.') && self.peek(1).is_some_and(|c| c.is_ascii_digit()) {
            repr.push(self.next_char().unwrap_or_default());
            while self.peek(0).is_some_and(|c| c.is_ascii_digit()) {
                repr.push(self.next_char().unwrap_or_default());
            }
        }
        if matches!(self.peek(0), Some('e' | 'E')) {
            let has_exp = match self.peek(1) {
                Some(c) if c.is_ascii_digit() => true,
                Some('+' | '-') => self.peek(2).is_some_and(|c| c.is_ascii_digit()),
                _ => false,
            };
            if has_exp {
                repr.push(self.next_char().unwrap_or_default());
                if matches!(self.peek(0), Some('+' | '-')) {
                    repr.push(self.next_char().unwrap_or_default());
                }
                while self.peek(0).is_some_and(|c| c.is_ascii_digit()) {
                    repr.push(self.next_char().unwrap_or_default());
                }
            }
        }
        let value = repr.parse().unwrap_or(0.0);
        (repr, value)
    }

    /// Après un nombre : `%`, dimension ou nombre nu.
    fn numeric(&mut self) -> Token {
        let (repr, value) = self.consume_number();
        if self.peek(0) == Some('%') {
            self.next_char();
            Token::Percentage { repr, value }
        } else if self.starts_ident() {
            let unit = self.consume_name();
            Token::Dimension { repr, value, unit }
        } else {
            Token::Number { repr, value }
        }
    }

    /// Chaîne `"`/`'` — `BadString` sur newline non échappé.
    fn consume_string(&mut self, ending: char) -> Token {
        let mut out = String::new();
        loop {
            match self.next_char() {
                None => return Token::Str(out), // EOF : fin tolérée.
                Some(c) if c == ending => return Token::Str(out),
                Some('\n' | '\r' | '\x0C') => {
                    self.unread('\n');
                    return Token::BadString;
                }
                Some('\\') => match self.next_char() {
                    Some('\n' | '\r' | '\x0C') | None => {} // continuation / EOF
                    Some(c) => {
                        self.unread(c);
                        out.push(self.consume_escape());
                    }
                },
                Some(c) => out.push(c),
            }
        }
    }

    /// `url(` non quoté — `BadUrl` sur caractère invalide.
    fn consume_url(&mut self) -> Token {
        // Saute le whitespace initial.
        while self.peek(0).is_some_and(is_ws) {
            self.next_char();
        }
        let mut out = String::new();
        loop {
            match self.next_char() {
                None | Some(')') => return Token::Url(out),
                Some(c) if is_ws(c) => {
                    // ws → seul `)` ou EOF sont valides après.
                    while self.peek(0).is_some_and(is_ws) {
                        self.next_char();
                    }
                    return match self.next_char() {
                        None | Some(')') => Token::Url(out),
                        _ => self.consume_bad_url(),
                    };
                }
                Some('"' | '\'' | '(') => return self.consume_bad_url(),
                Some('\\') => {
                    if self.valid_escape(0) {
                        out.push(self.consume_escape());
                    } else {
                        return self.consume_bad_url();
                    }
                }
                Some(c) => out.push(c),
            }
        }
    }

    /// Avale jusqu'au `)` fermant d'un mauvais url.
    fn consume_bad_url(&mut self) -> Token {
        loop {
            match self.next_char() {
                None | Some(')') => return Token::BadUrl,
                Some('\\') if self.valid_escape(0) => {
                    let _ = self.consume_escape();
                }
                _ => {}
            }
        }
    }

    /// `<!--` / `-->` si présents, sinon `Delim` du char entrée.
    /// Invariant : `first` est `<` ou `-` (déjà consommé).
    fn markup(&mut self, first: char) -> Token {
        match first {
            '<' => {
                if self.peek(0) == Some('!')
                    && self.peek(1) == Some('-')
                    && self.peek(2) == Some('-')
                {
                    self.next_char();
                    self.next_char();
                    self.next_char();
                    Token::Cdo
                } else {
                    Token::Delim('<')
                }
            }
            '-' => {
                if self.peek(0) == Some('-') && self.peek(1) == Some('>') {
                    self.next_char();
                    self.next_char();
                    Token::Cdc
                } else if self.starts_number() {
                    // `-5px` : le `-` fait partie du nombre.
                    self.unread('-');
                    self.numeric()
                } else if self.peek(0) == Some('-')
                    || self.peek(0).is_some_and(is_name_start)
                    || self.valid_escape(0)
                {
                    // `-foo` / `--bar` / `-\41` : ident.
                    self.unread('-');
                    Token::Ident(self.consume_name())
                } else {
                    Token::Delim('-')
                }
            }
            _ => Token::Delim(first),
        }
    }

    fn comment(&mut self) {
        // `/*` déjà consommé — mange jusqu'à `*/` ou EOF.
        let mut prev = '\0';
        while let Some(c) = self.next_char() {
            if prev == '*' && c == '/' {
                return;
            }
            prev = c;
        }
    }

    fn next_token(&mut self) -> Token {
        loop {
            match self.next_char() {
                None => return Token::Eof,
                Some('/') if self.peek(0) == Some('*') => {
                    self.next_char();
                    self.comment();
                }
                Some(c) if is_ws(c) => {
                    while self.peek(0).is_some_and(is_ws) {
                        self.next_char();
                    }
                    return Token::Whitespace;
                }
                Some('"') => return self.consume_string('"'),
                Some('\'') => return self.consume_string('\''),
                Some('#') => {
                    if self.peek(0).is_some_and(is_name_char) || self.valid_escape(0) {
                        let name = self.consume_name();
                        // `#` suivi d'un ident-forme id ?
                        let is_id = name.chars().next().is_some_and(is_name_start)
                            || name.starts_with('-')
                                && name.chars().nth(1).is_some_and(is_name_start);
                        return Token::Hash { value: name, is_id };
                    }
                    return Token::Delim('#');
                }
                Some('(') => return Token::OpenParen,
                Some(')') => return Token::CloseParen,
                Some('[') => return Token::OpenBracket,
                Some(']') => return Token::CloseBracket,
                Some('{') => return Token::OpenBrace,
                Some('}') => return Token::CloseBrace,
                Some(':') => return Token::Colon,
                Some(';') => return Token::Semicolon,
                Some(',') => return Token::Comma,
                Some('@') => {
                    if self.starts_ident() {
                        return Token::AtKeyword(self.consume_name());
                    }
                    return Token::Delim('@');
                }
                Some(c @ ('<' | '-')) => return self.markup(c),
                // Nombre décidé sur `c` (déjà consommé) : chiffre,
                // ou `+`/`.` si la suite forme un nombre. (`-` passe
                // par `markup` : CDC/ident/nombre y sont arbitrés.)
                Some(c) if c.is_ascii_digit() => {
                    self.unread(c);
                    return self.numeric();
                }
                Some(c @ ('+' | '.')) => {
                    // `+`/`.` ouvre un nombre si digit ou `.digit`
                    // suit — sinon Delim (spec "starts with a number").
                    let opens_number = self.peek(0).is_some_and(|d| d.is_ascii_digit())
                        || (c == '+'
                            && self.peek(0) == Some('.')
                            && self.peek(1).is_some_and(|d| d.is_ascii_digit()));
                    if opens_number {
                        self.unread(c);
                        return self.numeric();
                    }
                    return Token::Delim(c);
                }
                Some('\\') => {
                    if self.valid_escape(0) {
                        self.unread('\\');
                        let name = self.consume_name();
                        return Token::Ident(name);
                    }
                    return Token::Delim('\\');
                }
                Some(c) if is_name_start(c) => {
                    self.unread(c);
                    let name = self.consume_name();
                    // `u+`/`U+` unicode-range → ignoré (Delim),
                    // url( → Function ou Url.
                    if self.peek(0) == Some('(') {
                        self.next_char();
                        if name.eq_ignore_ascii_case("url") {
                            // url( "…" ) quoté = Function(url) ;
                            // sinon url-token (spec).
                            while self.peek(0).is_some_and(is_ws) {
                                self.next_char();
                            }
                            return match self.peek(0) {
                                Some('"' | '\'') => Token::Function(name),
                                _ => self.consume_url(),
                            };
                        }
                        return Token::Function(name);
                    }
                    return Token::Ident(name);
                }
                Some(c) => return Token::Delim(c),
            }
        }
    }
}
