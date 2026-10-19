// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Lexer for the `dillo-js` ES3-flavoured subset.
//!
//! Tokens are produced eagerly into a `Vec<Token>`; each token records
//! its position and whether a line break preceded it, which the parser
//! needs for the minimal automatic-semicolon-insertion rules (`return\n`,
//! postfix `++`/`--`, and statement termination without `;`).

use crate::error::{Error, Result};

/// Reserved words recognised by the lexer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kw {
    /// `var`
    Var,
    /// `function`
    Function,
    /// `return`
    Return,
    /// `if`
    If,
    /// `else`
    Else,
    /// `while`
    While,
    /// `do`
    Do,
    /// `for`
    For,
    /// `in`
    In,
    /// `break`
    Break,
    /// `continue`
    Continue,
    /// `new`
    New,
    /// `this`
    This,
    /// `typeof`
    Typeof,
    /// `void`
    Void,
    /// `delete`
    Delete,
    /// `instanceof`
    Instanceof,
    /// `throw`
    Throw,
    /// `try`
    Try,
    /// `catch`
    Catch,
    /// `finally`
    Finally,
    /// `true`
    True,
    /// `false`
    False,
    /// `null`
    Null,
    /// `switch`
    Switch,
    /// `case`
    Case,
    /// `default`
    Default,
    /// `let`
    Let,
    /// `const`
    Const,
    /// `class`
    Class,
    /// `extends`
    Extends,
    /// `super`
    Super,
    /// `import`
    Import,
    /// `export`
    Export,
    /// `with`
    With,
    /// A real ECMAScript keyword the engine does not implement yet
    /// (`debugger`, `enum`…).
    Unsupported(&'static str),
}

/// Punctuation and operators.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum P {
    /// `(`
    LParen,
    /// `)`
    RParen,
    /// `{`
    LBrace,
    /// `}`
    RBrace,
    /// `[`
    LBracket,
    /// `]`
    RBracket,
    /// `;`
    Semi,
    /// `,`
    Comma,
    /// `.`
    Dot,
    /// `?`
    Question,
    /// `:`
    Colon,
    /// `+`
    Plus,
    /// `-`
    Minus,
    /// `*`
    Star,
    /// `/`
    Slash,
    /// `%`
    Percent,
    /// `++`
    Inc,
    /// `--`
    Dec,
    /// `<`
    Lt,
    /// `>`
    Gt,
    /// `<=`
    Le,
    /// `>=`
    Ge,
    /// `==`
    EqEq,
    /// `!=`
    Ne,
    /// `===`
    EqEqEq,
    /// `!==`
    NeNe,
    /// `&`
    Amp,
    /// `|`
    Pipe,
    /// `^`
    Caret,
    /// `!`
    Bang,
    /// `~`
    Tilde,
    /// `&&`
    AmpAmp,
    /// `||`
    PipePipe,
    /// `<<`
    LtLt,
    /// `>>`
    GtGt,
    /// `>>>`
    GtGtGt,
    /// `=`
    Assign,
    /// `+=`
    PlusEq,
    /// `-=`
    MinusEq,
    /// `*=`
    StarEq,
    /// `/=`
    SlashEq,
    /// `%=`
    PercentEq,
    /// `&=`
    AmpEq,
    /// `|=`
    PipeEq,
    /// `^=`
    CaretEq,
    /// `<<=`
    LtLtEq,
    /// `>>=`
    GtGtEq,
    /// `>>>=`
    GtGtGtEq,
    /// `=>`
    Arrow,
    /// `?.`
    QDot,
    /// `??`
    QQ,
    /// `??=`
    QQEq,
    /// `&&=`
    AmpAmpEq,
    /// `||=`
    PipePipeEq,
    /// `...`
    Ellipsis,
    /// `**`
    StarStar,
    /// `**=`
    StarStarEq,
}

/// Token kinds.
#[derive(Debug, Clone, PartialEq)]
pub enum Tok {
    /// End of input.
    Eof,
    /// Identifier (or unreserved word).
    Ident(String),
    /// Reserved word.
    Kw(Kw),
    /// Numeric literal (already converted to `f64`).
    Num(f64),
    /// `BigInt` literal `123n` / `0xFn` / `0o17n` / `0b101n` —
    /// digits normalisés en décimal (le lexer connaît la base).
    Big(String),
    /// String literal (escapes already decoded).
    Str(String),
    /// Punctuation / operator.
    P(P),
    /// Template literal: literal chunks interleaved with `${…}`
    /// substitutions (each stored as its own token slice, parsed
    /// lazily by the parser).
    Tpl(Vec<TplPart>),
    /// `/pat/flags` regex literal — only lexed where an operand is
    /// expected (the lexer tracks operand/operator context).
    Regex {
        /// Pattern source (unescaped form, between the slashes).
        body: String,
        /// Flag letters after the closing slash.
        flags: String,
    },
}

/// One piece of a template literal.
#[derive(Debug, Clone, PartialEq)]
pub enum TplPart {
    /// Literal text (escapes already decoded).
    Str(String),
    /// Tokens of a `${…}` substitution (terminated by `Tok::Eof`).
    Expr(Vec<Token>),
}

/// One token plus the source trivia the parser cares about.
#[derive(Debug, Clone, PartialEq)]
pub struct Token {
    /// The token itself.
    pub tok: Tok,
    /// 1-based line.
    pub line: usize,
    /// 1-based column.
    pub col: usize,
    /// Whether at least one line break occurred before this token.
    pub newline_before: bool,
    /// Literal carries a legacy octal form (`010`, `08`, `\1`) —
    /// tolerated in sloppy mode, `SyntaxError` in strict mode
    /// (checked by the parser, which owns the directive prologue).
    pub legacy_octal: bool,
}

/// Lex `src` into a token vector terminated by a `Tok::Eof` token.
pub fn lex(src: &str) -> Result<Vec<Token>> {
    Lexer {
        chars: src.chars().collect(),
        pos: 0,
        line: 1,
        col: 1,
        legacy_octal: false,
    }
    .run()
}

struct Lexer {
    chars: Vec<char>,
    pos: usize,
    line: usize,
    col: usize,
    /// Set while producing the current token (see `Token::legacy_octal`).
    legacy_octal: bool,
}

impl Lexer {
    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    fn peek_at(&self, ahead: usize) -> Option<char> {
        self.chars.get(self.pos + ahead).copied()
    }

    fn bump(&mut self) -> Option<char> {
        let c = self.peek()?;
        self.pos += 1;
        if c == '\r' && self.peek() == Some('\n') {
            // CRLF = un seul terminateur : le `\n` est consommé par
            // l'appelant sans doubler le compte.
            self.pos += 1;
            self.line += 1;
            self.col = 1;
            return Some('\n');
        }
        if is_line_term(c) {
            self.line += 1;
            self.col = 1;
        } else {
            self.col += 1;
        }
        Some(c)
    }

    fn err(&self, message: impl Into<String>) -> Error {
        Error::Syntax {
            line: self.line,
            col: self.col,
            message: message.into(),
        }
    }

    fn run(&mut self) -> Result<Vec<Token>> {
        let mut out = Vec::new();
        // `/` is division after an operand, a regex literal start
        // otherwise — the classic heuristic (no parser feedback).
        let mut expect_operand = true;
        loop {
            let newline_before = self.skip_trivia()?;
            let (line, col) = (self.line, self.col);
            self.legacy_octal = false;
            let tok = match self.peek() {
                None => Tok::Eof,
                Some(c) if is_ident_start(c) => self.ident()?,
                Some('\\') => self.ident()?,
                Some(c) if c.is_ascii_digit() => self.number()?,
                Some('.') if self.peek_at(1).is_some_and(|c| c.is_ascii_digit()) => {
                    self.number()?
                }
                Some('"' | '\'') => Tok::Str(self.string()?),
                Some('`') => self.template()?,
                Some('/') if expect_operand => self.regex_lit()?,
                Some(_) => Tok::P(self.punct()?),
            };
            expect_operand = !ends_operand(&tok);
            let is_eof = tok == Tok::Eof;
            out.push(Token {
                tok,
                line,
                col,
                newline_before,
                legacy_octal: self.legacy_octal,
            });
            if is_eof {
                return Ok(out);
            }
        }
    }

    /// `/body/flags` — `in_class` keeps `[/]` inside `[…]` from
    /// closing the literal. Flags are identifier chars.
    fn regex_lit(&mut self) -> Result<Tok> {
        self.bump(); // `/`
        let mut body = String::new();
        let mut in_class = false;
        loop {
            match self.bump() {
                None => return Err(self.err("unterminated regex literal")),
                Some(c) if is_line_term(c) => {
                    return Err(self.err("line terminator in regex literal"));
                }
                Some('\\') => {
                    body.push('\\');
                    match self.bump() {
                        None => return Err(self.err("unterminated regex literal")),
                        Some(c) if is_line_term(c) => {
                            return Err(self.err("line terminator in regex literal"));
                        }
                        Some(c) => body.push(c),
                    }
                }
                Some('[') => {
                    in_class = true;
                    body.push('[');
                }
                Some(']') => {
                    in_class = false;
                    body.push(']');
                }
                Some('/') if !in_class => break,
                Some(c) => body.push(c),
            }
        }
        let mut flags = String::new();
        while self.peek().is_some_and(is_ident_part) {
            flags.push(self.bump().unwrap_or_default());
        }
        Ok(Tok::Regex { body, flags })
    }

    /// Skip whitespace and comments. Returns whether a newline was crossed.
    fn skip_trivia(&mut self) -> Result<bool> {
        let mut saw_newline = false;
        // `-->` n'est un commentaire (Annex B) qu'en début de ligne
        // — c.-à-d. rien d'autre que du trivia depuis le dernier
        // terminateur (ou le début de source).
        let mut at_bol = self.pos == 0;
        loop {
            match self.peek() {
                // WhiteSpace ES = Unicode White_Space + U+FEFF
                // (ZWNBSP — Cf, hors `is_whitespace` Rust).
                Some(c) if c.is_whitespace() || c == '\u{feff}' => {
                    saw_newline |= is_line_term(c);
                    self.bump();
                }
                Some('/') if self.peek_at(1) == Some('/') => {
                    self.skip_line_comment();
                }
                Some('<')
                    if self.peek_at(1) == Some('!')
                        && self.peek_at(2) == Some('-')
                        && self.peek_at(3) == Some('-') =>
                {
                    self.skip_line_comment();
                }
                Some('-')
                    if at_bol && self.peek_at(1) == Some('-') && self.peek_at(2) == Some('>') =>
                {
                    self.skip_line_comment();
                }
                Some('/') if self.peek_at(1) == Some('*') => {
                    self.bump();
                    self.bump();
                    loop {
                        match self.bump() {
                            None => {
                                return Err(self.err("unterminated multi-line comment"));
                            }
                            Some(c) if is_line_term(c) => {
                                saw_newline = true;
                            }
                            Some('*') if self.peek() == Some('/') => {
                                self.bump();
                                break;
                            }
                            Some(_) => {}
                        }
                    }
                }
                _ => return Ok(saw_newline),
            }
            at_bol |= saw_newline;
        }
    }

    /// `// …` / `<!-- …` / `--> …` — jusqu'au prochain terminateur
    /// (consommé par l'arme whitespace au tour suivant).
    fn skip_line_comment(&mut self) {
        while let Some(c) = self.peek() {
            if is_line_term(c) {
                break;
            }
            self.bump();
        }
    }

    /// `\uXXXX` ou `\u{X…}` — utilisé en identifiant et en chaîne.
    /// `u` déjà vérifié, `\` non encore consommé.
    fn unicode_escape(&mut self) -> Result<char> {
        self.bump(); // `\`
        self.bump(); // `u`
        let v = if self.peek() == Some('{') {
            self.bump();
            let mut v = 0u32;
            let mut n = 0usize;
            while let Some(c) = self.peek() {
                if c == '}' {
                    break;
                }
                let Some(d) = c.to_digit(16) else {
                    return Err(self.err("malformed unicode escape"));
                };
                v = v.wrapping_mul(16).wrapping_add(d);
                n += 1;
                if n > 7 || v > 0x0010_FFFF {
                    return Err(self.err("unicode escape out of range"));
                }
                self.bump();
            }
            if self.bump() != Some('}') || n == 0 {
                return Err(self.err("malformed unicode escape"));
            }
            v
        } else {
            self.hex_digits(4)?
        };
        char::from_u32(v).ok_or_else(|| self.err("unicode escape out of range"))
    }

    /// Identifiant — accepte les échappes `\uXXXX`/`\u{…}` (valeur
    /// décodée) ; un mot réservé produit par échappe est une
    /// erreur de syntaxe (spec : les réservés ne s'écrivent pas
    /// en échappés).
    fn ident(&mut self) -> Result<Tok> {
        let mut s = String::new();
        let mut escaped = false;
        loop {
            match self.peek() {
                Some('\\') => {
                    if self.peek_at(1) != Some('u') {
                        return Err(self.err("unexpected `\\`"));
                    }
                    escaped = true;
                    let c = self.unicode_escape()?;
                    let ok = if s.is_empty() {
                        is_ident_start(c)
                    } else {
                        is_ident_part(c)
                    };
                    if !ok {
                        return Err(self.err(format!("invalid escape `{c}` in identifier")));
                    }
                    s.push(c);
                }
                Some(c) if s.is_empty() && is_ident_start(c) => {
                    s.push(c);
                    self.bump();
                }
                Some(c) if !s.is_empty() && is_ident_part(c) => {
                    s.push(c);
                    self.bump();
                }
                _ => break,
            }
        }
        match keyword(&s) {
            Some(_) if escaped => Err(self.err("reserved word cannot be escaped")),
            Some(kw) => Ok(Tok::Kw(kw)),
            None => Ok(Tok::Ident(s)),
        }
    }

    fn number(&mut self) -> Result<Tok> {
        let start = self.pos;
        // `0x`/`0b`/`0o` prefixed literals.
        if self.peek() == Some('0') {
            if let Some(k) = self.peek_at(1) {
                let radix = match k {
                    'x' | 'X' => 16,
                    'b' | 'B' => 2,
                    'o' | 'O' => 8,
                    _ => 0,
                };
                if radix > 0 {
                    self.bump();
                    self.bump();
                    let ds = self.pos;
                    let dig = |c: char| match radix {
                        16 => c.is_ascii_hexdigit(),
                        8 => matches!(c, '0'..='7'),
                        _ => matches!(c, '0' | '1'),
                    };
                    while self.peek().is_some_and(dig) {
                        self.bump();
                    }
                    let s: String = self.chars[ds..self.pos].iter().collect();
                    if s.is_empty() {
                        return Err(self.err("malformed numeric literal"));
                    }
                    // Suffixe `n` → BigInt (`0xFn`, `0o17n`, `0b101n`).
                    if self.peek() == Some('n') {
                        self.bump();
                        if self
                            .peek()
                            .is_some_and(|c| is_ident_start(c) || c.is_ascii_digit())
                        {
                            return Err(self.err("identifier directly after number"));
                        }
                        return match num_bigint::BigInt::parse_bytes(s.as_bytes(), radix) {
                            Some(b) => Ok(Tok::Big(b.to_string())),
                            None => Err(self.err("malformed bigint literal")),
                        };
                    }
                    if self.peek().is_some_and(is_ident_start) {
                        return Err(self.err("identifier directly after number"));
                    }
                    return match u64::from_str_radix(&s, radix) {
                        Ok(v) => Ok(Tok::Num(v as f64)),
                        Err(_) => Err(self.err("malformed numeric literal")),
                    };
                }
            }
            // `0[0-7]+` legacy octal et `0[89]` NonOctalDecimal :
            // flaggés pour le rejet strict côté parseur.
            if self.peek_at(1).is_some_and(|c| c.is_ascii_digit()) {
                let mut i = 1;
                let mut all_octal = true;
                while let Some(c) = self.peek_at(i) {
                    if c.is_ascii_digit() {
                        all_octal &= matches!(c, '0'..='7');
                        i += 1;
                    } else {
                        break;
                    }
                }
                let follows_dot =
                    self.peek_at(i) == Some('.') || matches!(self.peek_at(i), Some('e' | 'E'));
                if !follows_dot {
                    self.legacy_octal = true;
                }
                if all_octal && !follows_dot {
                    while self.peek().is_some_and(|c| matches!(c, '0'..='7')) {
                        self.bump();
                    }
                    if self.peek().is_some_and(is_ident_start) {
                        return Err(self.err("identifier directly after number"));
                    }
                    let s: String = self.chars[start..self.pos].iter().collect();
                    return match u64::from_str_radix(&s, 8) {
                        Ok(v) => Ok(Tok::Num(v as f64)),
                        Err(_) => Err(self.err("malformed octal literal")),
                    };
                }
            }
        }
        while self.peek().is_some_and(|c| c.is_ascii_digit()) {
            self.bump();
        }
        // Suffixe `n` → BigInt décimal — pas de point ni exposant,
        // et `08n`/`010n` (NonOctalDecimal/octale legacy) rejetés.
        if self.peek() == Some('n') {
            let lit: String = self.chars[start..self.pos].iter().collect();
            if lit.len() > 1 && lit.starts_with('0') {
                return Err(self.err("legacy octal bigint literal"));
            }
            self.bump();
            if self
                .peek()
                .is_some_and(|c| is_ident_start(c) || c.is_ascii_digit())
            {
                return Err(self.err("identifier directly after number"));
            }
            return match num_bigint::BigInt::parse_bytes(lit.as_bytes(), 10) {
                Some(b) => Ok(Tok::Big(b.to_string())),
                None => Err(self.err("malformed bigint literal")),
            };
        }
        if self.peek() == Some('.') {
            self.bump();
            while self.peek().is_some_and(|c| c.is_ascii_digit()) {
                self.bump();
            }
        }
        if matches!(self.peek(), Some('e' | 'E')) {
            let save = self.pos;
            self.bump();
            if matches!(self.peek(), Some('+' | '-')) {
                self.bump();
            }
            if self.peek().is_some_and(|c| c.is_ascii_digit()) {
                while self.peek().is_some_and(|c| c.is_ascii_digit()) {
                    self.bump();
                }
            } else {
                self.pos = save; // lone 'e' — not part of the literal
            }
        }
        // NumericLiteral ne peut pas être suivi d'un identifiant
        // (`3in` → SyntaxError ; `3 .toString` reste valide).
        if self.peek().is_some_and(is_ident_start) {
            return Err(self.err("identifier directly after number"));
        }
        let s: String = self.chars[start..self.pos].iter().collect();
        s.parse::<f64>()
            .map(Tok::Num)
            .map_err(|_| self.err(format!("malformed number literal `{s}`")))
    }

    fn string(&mut self) -> Result<String> {
        let quote = self.bump(); // opening quote
        let mut out = String::new();
        loop {
            match self.bump() {
                None => return Err(self.err("unterminated string literal")),
                // ES2019 : LS/PS bruts sont permis ; LF/CR non.
                Some('\n' | '\r') => return Err(self.err("newline in string literal")),
                Some(c) if Some(c) == quote => return Ok(out),
                Some('\\') => match self.bump() {
                    None => return Err(self.err("unterminated string escape")),
                    Some(c) if is_line_term(c) => {} // line continuation
                    Some('n') => out.push('\n'),
                    Some('t') => out.push('\t'),
                    Some('r') => out.push('\r'),
                    Some('b') => out.push('\u{8}'),
                    Some('f') => out.push('\u{c}'),
                    Some('v') => out.push('\u{b}'),
                    // LegacyOctalEscapeSequence (sloppy) : `\0` NUL
                    // si non suivi d'un chiffre ; `\N`→`\NNN` octal —
                    // 3 chiffres si tête ∈ 0..=3, 2 sinon. `\8`/`\9`
                    // sont NonEscapeCharacter → littéraux.
                    Some(d @ '0'..='7') => {
                        // `\0` seul = NUL légal même en strict ;
                        // flag uniquement si annexe B effective.
                        if d != '0' || self.peek().is_some_and(|c| c.is_ascii_digit()) {
                            self.legacy_octal = true;
                        }
                        let mut v = d.to_digit(8).unwrap_or(0);
                        if d != '0' || self.peek().is_some_and(|c| ('0'..='7').contains(&c)) {
                            let max = if d <= '3' { 3 } else { 2 };
                            for _ in 1..max {
                                match self.peek() {
                                    Some(c @ '0'..='7') => {
                                        v = v * 8 + c.to_digit(8).unwrap_or(0);
                                        self.bump();
                                    }
                                    _ => break,
                                }
                            }
                        }
                        out.push(char::from_u32(v).unwrap_or('\u{fffd}'));
                    }
                    Some('x') => {
                        let v = self.hex_digits(2)?;
                        out.push(char::from_u32(v).unwrap_or('\u{fffd}'));
                    }
                    Some('u') => out.push(self.read_unicode_escape()?),
                    // `\8`/`\9` : NonOctalDecimalEscapeSequence —
                    // littéraux en sloppy, interdits en strict.
                    Some(d @ ('8' | '9')) => {
                        self.legacy_octal = true;
                        out.push(d);
                    }
                    Some(c) => out.push(c),
                },
                Some(c) => out.push(c),
            }
        }
    }

    /// `` `text ${expr} …` `` — literal chunks + `${…}` inner sources,
    /// each re-lexed into its own token slice (positions inside a
    /// substitution are relative to it — diagnostics only).
    fn template(&mut self) -> Result<Tok> {
        self.bump(); // opening backquote
        let mut parts = Vec::new();
        let mut cur = String::new();
        loop {
            match self.bump() {
                None => return Err(self.err("unterminated template literal")),
                Some('`') => {
                    parts.push(TplPart::Str(cur));
                    return Ok(Tok::Tpl(parts));
                }
                Some('$') if self.peek() == Some('{') => {
                    self.bump();
                    parts.push(TplPart::Str(std::mem::take(&mut cur)));
                    let src = self.template_expr_src()?;
                    parts.push(TplPart::Expr(lex(&src)?));
                }
                Some('\\') => match self.bump() {
                    None => return Err(self.err("unterminated template escape")),
                    Some('n') => cur.push('\n'),
                    Some('t') => cur.push('\t'),
                    Some('r') => cur.push('\r'),
                    Some('b') => cur.push('\u{8}'),
                    Some('f') => cur.push('\u{c}'),
                    Some('v') => cur.push('\u{b}'),
                    Some('0') => cur.push('\0'),
                    Some('x') => {
                        let v = self.hex_digits(2)?;
                        cur.push(char::from_u32(v).unwrap_or('\u{fffd}'));
                    }
                    Some('u') => cur.push(self.read_unicode_escape()?),
                    Some('`') => cur.push('`'),
                    Some('$') => cur.push('$'),
                    Some('\n') => {} // line continuation
                    Some(c) => cur.push(c),
                },
                Some(c) => cur.push(c),
            }
        }
    }

    /// Source of a `${…}` substitution: consumes up to the matching
    /// `}` — nested `{}`, quoted strings and nested templates are
    /// balanced so `}` inside them does not close the substitution.
    fn template_expr_src(&mut self) -> Result<String> {
        let start = self.pos;
        let mut depth = 1usize;
        loop {
            match self.peek() {
                None => return Err(self.err("unterminated template substitution")),
                Some('{') => {
                    depth += 1;
                    self.bump();
                }
                Some('}') => {
                    self.bump();
                    depth -= 1;
                    if depth == 0 {
                        return Ok(self.chars[start..self.pos - 1].iter().collect());
                    }
                }
                Some('\'' | '"') => self.skip_quoted()?,
                Some('`') => self.skip_nested_template()?,
                Some(_) => {
                    self.bump();
                }
            }
        }
    }

    /// Skip `'…'`/`"…"` inside a `${…}` scan (escapes honoured).
    fn skip_quoted(&mut self) -> Result<()> {
        let quote = self.bump();
        loop {
            match self.bump() {
                None => return Err(self.err("unterminated string in template")),
                Some('\\') => {
                    self.bump();
                }
                Some(c) if Some(c) == quote => return Ok(()),
                Some(_) => {}
            }
        }
    }

    /// Skip a nested `` `…` `` inside a `${…}` scan; its own `${…}`
    /// substitutions are balanced recursively.
    fn skip_nested_template(&mut self) -> Result<()> {
        self.bump(); // `
        loop {
            match self.peek() {
                None => return Err(self.err("unterminated nested template")),
                Some('`') => {
                    self.bump();
                    return Ok(());
                }
                Some('\\') => {
                    self.bump();
                    self.bump();
                }
                Some('$') if self.peek_at(1) == Some('{') => {
                    self.bump();
                    self.bump();
                    self.template_expr_src()?;
                }
                Some('\'' | '"') => self.skip_quoted()?,
                Some(_) => {
                    self.bump();
                }
            }
        }
    }

    fn hex_digits(&mut self, n: usize) -> Result<u32> {
        let mut v = 0u32;
        for _ in 0..n {
            match self.bump().and_then(|c| c.to_digit(16)) {
                Some(d) => v = v * 16 + d,
                None => return Err(self.err("malformed hex escape")),
            }
        }
        Ok(v)
    }

    /// `\u{XXXX…}` ou `\uXXXX` — le `u` est déjà consommé.
    /// `\uD8xx\uDCxx` fusionne la paire de surrogates en un seul
    /// code point (les chaînes JS sont UTF-16 ; on stocke du
    /// `char` — un surrogate seul dégrade en U+FFFD).
    fn read_unicode_escape(&mut self) -> Result<char> {
        if self.peek() == Some('{') {
            self.bump();
            let mut v = 0u32;
            let mut n = 0usize;
            while let Some(c) = self.peek() {
                if c == '}' {
                    break;
                }
                let Some(d) = c.to_digit(16) else {
                    return Err(self.err("malformed unicode escape"));
                };
                v = v.wrapping_mul(16).wrapping_add(d);
                n += 1;
                self.bump();
            }
            if n == 0 || v > 0x0010_FFFF || self.bump() != Some('}') {
                return Err(self.err("unicode escape out of range"));
            }
            return Ok(char::from_u32(v).unwrap_or('\u{fffd}'));
        }
        let v = self.hex_digits(4)?;
        if (0xd800..0xdc00).contains(&v)
            && self.peek() == Some('\\')
            && self.peek_at(1) == Some('u')
        {
            let save = self.pos;
            self.bump();
            self.bump();
            let lo = self.hex_digits(4).unwrap_or(u32::MAX);
            if (0xdc00..0xe000).contains(&lo) {
                let cp = 0x1_0000 + ((v - 0xd800) << 10) + (lo - 0xdc00);
                return Ok(char::from_u32(cp).unwrap_or('\u{fffd}'));
            }
            self.pos = save;
            return Ok('\u{fffd}');
        }
        Ok(char::from_u32(v).unwrap_or('\u{fffd}'))
    }

    fn punct(&mut self) -> Result<P> {
        let c1 = self.peek().unwrap_or('\0');
        let c2 = self.peek_at(1).unwrap_or('\0');
        let c3 = self.peek_at(2).unwrap_or('\0');
        let c4 = self.peek_at(3).unwrap_or('\0');
        // Longest match first.
        let (len, p) = match (c1, c2, c3, c4) {
            ('>', '>', '>', '=') => (4, P::GtGtGtEq),
            ('.', '.', '.', _) => (3, P::Ellipsis),
            ('=', '=', '=', _) => (3, P::EqEqEq),
            ('!', '=', '=', _) => (3, P::NeNe),
            ('>', '>', '>', _) => (3, P::GtGtGt),
            ('<', '<', '=', _) => (3, P::LtLtEq),
            ('>', '>', '=', _) => (3, P::GtGtEq),
            ('?', '?', '=', _) => (3, P::QQEq),
            ('&', '&', '=', _) => (3, P::AmpAmpEq),
            ('|', '|', '=', _) => (3, P::PipePipeEq),
            ('*', '*', '=', _) => (3, P::StarStarEq),
            ('=', '>', _, _) => (2, P::Arrow),
            ('?', '?', _, _) => (2, P::QQ),
            ('?', '.', _, _) if !c3.is_ascii_digit() => (2, P::QDot),
            ('*', '*', _, _) => (2, P::StarStar),
            ('+', '+', _, _) => (2, P::Inc),
            ('-', '-', _, _) => (2, P::Dec),
            ('&', '&', _, _) => (2, P::AmpAmp),
            ('|', '|', _, _) => (2, P::PipePipe),
            ('=', '=', _, _) => (2, P::EqEq),
            ('!', '=', _, _) => (2, P::Ne),
            ('<', '=', _, _) => (2, P::Le),
            ('>', '=', _, _) => (2, P::Ge),
            ('<', '<', _, _) => (2, P::LtLt),
            ('>', '>', _, _) => (2, P::GtGt),
            ('+', '=', _, _) => (2, P::PlusEq),
            ('-', '=', _, _) => (2, P::MinusEq),
            ('*', '=', _, _) => (2, P::StarEq),
            ('/', '=', _, _) => (2, P::SlashEq),
            ('%', '=', _, _) => (2, P::PercentEq),
            ('&', '=', _, _) => (2, P::AmpEq),
            ('|', '=', _, _) => (2, P::PipeEq),
            ('^', '=', _, _) => (2, P::CaretEq),
            _ => {
                let p = match c1 {
                    '(' => P::LParen,
                    ')' => P::RParen,
                    '{' => P::LBrace,
                    '}' => P::RBrace,
                    '[' => P::LBracket,
                    ']' => P::RBracket,
                    ';' => P::Semi,
                    ',' => P::Comma,
                    '.' => P::Dot,
                    '?' => P::Question,
                    ':' => P::Colon,
                    '+' => P::Plus,
                    '-' => P::Minus,
                    '*' => P::Star,
                    '/' => P::Slash,
                    '%' => P::Percent,
                    '<' => P::Lt,
                    '>' => P::Gt,
                    '=' => P::Assign,
                    '!' => P::Bang,
                    '~' => P::Tilde,
                    '&' => P::Amp,
                    '|' => P::Pipe,
                    '^' => P::Caret,
                    c => return Err(self.err(format!("unexpected character `{c}`"))),
                };
                (1, p)
            }
        };
        for _ in 0..len {
            self.bump();
        }
        Ok(p)
    }
}

/// Does this token end an operand position — i.e. a following `/`
/// is division, not a regex literal. `}` counts as operand-end
/// (object literals more common than block-then-regex code).
fn ends_operand(t: &Tok) -> bool {
    matches!(
        t,
        Tok::Ident(_)
            | Tok::Num(_)
            | Tok::Big(_)
            | Tok::Str(_)
            | Tok::Tpl(_)
            | Tok::Regex { .. }
            | Tok::Kw(Kw::This | Kw::True | Kw::False | Kw::Null)
            | Tok::P(P::RParen | P::RBracket | P::RBrace | P::Inc | P::Dec)
    )
}

/// `ID_Start` ≙ propriété Unicode `ID_Start` + `$`/`_` (grammar
/// ECMAScript). Implémenté sur `unicode-ident` (tables UAX #31
/// compilées, suivies à jour par Unicode) : `char::is_alphabetic`
/// accepte trop (`Lm` `Pattern_Syntax` comme U+2E2F) et trop peu
/// (`Other_ID_Start`).
fn is_ident_start(c: char) -> bool {
    c == '_'
        || c == '$'
        || unicode_ident::is_xid_start(c)
        // Other_ID_Start : hors XID mais requis par la grammaire
        // ES (℘ ℮ ゛ ゜ + voyelles mongoles).
        || matches!(
            c,
            '\u{1885}' | '\u{1886}' | '\u{2118}' | '\u{212e}' | '\u{309b}' | '\u{309c}'
        )
}

/// `ID_Continue` ≙ `ID_Start` + `Other_ID_Continue` + ZWNJ/ZWJ.
fn is_ident_part(c: char) -> bool {
    // ID_Start ⊆ ID_Continue — `is_ident_start` est réutilisé
    // (inclut `_`/`$`/`Other_ID_Start`).
    is_ident_start(c)
        || c == '\u{200c}'
        || c == '\u{200d}'
        || unicode_ident::is_xid_continue(c)
        || matches!(
            c,
            '\u{00b7}' | '\u{0387}' | '\u{05f3}' | '\u{19da}' | '\u{30fb}' | '\u{1369}'
                ..='\u{1371}'
        )
}

/// Wrappers `pub(crate)` pour `regexp.rs` — les noms de groupes
/// capturants sont des `IdentifierName` (mêmes tables UAX #31).
pub(crate) fn is_ident_start_pub(c: char) -> bool {
    is_ident_start(c)
}

/// Voir [`is_ident_start_pub`].
pub(crate) fn is_ident_part_pub(c: char) -> bool {
    is_ident_part(c)
}

/// Terminateur de ligne ECMAScript : LF, CR, LS, PS — pilote
/// l'ASI (`newline_before`) et le comptage de lignes. CRLF ne
/// compte qu'une fois (géré par le consommateur).
fn is_line_term(c: char) -> bool {
    matches!(c, '\n' | '\r' | '\u{2028}' | '\u{2029}')
}

fn keyword(s: &str) -> Option<Kw> {
    use Kw::{
        Break, Case, Catch, Class, Const, Continue, Default, Delete, Do, Else, Export, Extends,
        False, Finally, For, Function, If, Import, In, Instanceof, Let, New, Null, Return, Super,
        Switch, This, Throw, True, Try, Typeof, Unsupported as U, Var, Void, While, With,
    };
    Some(match s {
        "var" => Var,
        "function" => Function,
        "return" => Return,
        "if" => If,
        "else" => Else,
        "while" => While,
        "do" => Do,
        "for" => For,
        "in" => In,
        "break" => Break,
        "continue" => Continue,
        "new" => New,
        "this" => This,
        "typeof" => Typeof,
        "void" => Void,
        "delete" => Delete,
        "instanceof" => Instanceof,
        "throw" => Throw,
        "try" => Try,
        "catch" => Catch,
        "finally" => Finally,
        "true" => True,
        "false" => False,
        "null" => Null,
        "switch" => Switch,
        "case" => Case,
        "default" => Default,
        "let" => Let,
        "const" => Const,
        "class" => Class,
        "extends" => Extends,
        "super" => Super,
        "with" => With,
        "debugger" => U("debugger"),
        "enum" => U("enum"),
        "import" => Import,
        "export" => Export,
        _ => return None,
    })
}
