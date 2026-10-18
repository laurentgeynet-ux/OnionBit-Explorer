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
    /// A real ECMAScript keyword the engine does not implement yet
    /// (`with`, `import`, `yield`…).
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
}

/// Lex `src` into a token vector terminated by a `Tok::Eof` token.
pub fn lex(src: &str) -> Result<Vec<Token>> {
    Lexer {
        chars: src.chars().collect(),
        pos: 0,
        line: 1,
        col: 1,
    }
    .run()
}

struct Lexer {
    chars: Vec<char>,
    pos: usize,
    line: usize,
    col: usize,
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
        if c == '\n' {
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
            let tok = match self.peek() {
                None => Tok::Eof,
                Some(c) if is_ident_start(c) => self.ident(),
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
                None | Some('\n') => return Err(self.err("unterminated regex literal")),
                Some('\\') => {
                    body.push('\\');
                    match self.bump() {
                        None | Some('\n') => return Err(self.err("unterminated regex literal")),
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
        loop {
            match self.peek() {
                Some(c) if c.is_whitespace() => {
                    saw_newline |= c == '\n';
                    self.bump();
                }
                Some('/') if self.peek_at(1) == Some('/') => {
                    while let Some(c) = self.peek() {
                        if c == '\n' {
                            break;
                        }
                        self.bump();
                    }
                }
                Some('/') if self.peek_at(1) == Some('*') => {
                    self.bump();
                    self.bump();
                    loop {
                        match self.bump() {
                            None => return Ok(saw_newline), // unterminated → EOF
                            Some('\n') => saw_newline = true,
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
        }
    }

    fn ident(&mut self) -> Tok {
        let start = self.pos;
        while self.peek().is_some_and(is_ident_part) {
            self.bump();
        }
        let s: String = self.chars[start..self.pos].iter().collect();
        match keyword(&s) {
            Some(kw) => Tok::Kw(kw),
            None => Tok::Ident(s),
        }
    }

    fn number(&mut self) -> Result<Tok> {
        let start = self.pos;
        if self.peek() == Some('0') && matches!(self.peek_at(1), Some('x' | 'X')) {
            self.bump();
            self.bump();
            while self.peek().is_some_and(|c| c.is_ascii_hexdigit()) {
                self.bump();
            }
            let s: String = self.chars[start + 2..self.pos].iter().collect();
            return match u64::from_str_radix(&s, 16) {
                Ok(v) => Ok(Tok::Num(v as f64)),
                Err(_) => Err(self.err("malformed hex literal")),
            };
        }
        while self.peek().is_some_and(|c| c.is_ascii_digit()) {
            self.bump();
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
                Some('\n') => return Err(self.err("newline in string literal")),
                Some(c) if Some(c) == quote => return Ok(out),
                Some('\\') => match self.bump() {
                    None => return Err(self.err("unterminated string escape")),
                    Some('\n') => {} // line continuation
                    Some('n') => out.push('\n'),
                    Some('t') => out.push('\t'),
                    Some('r') => out.push('\r'),
                    Some('b') => out.push('\u{8}'),
                    Some('f') => out.push('\u{c}'),
                    Some('v') => out.push('\u{b}'),
                    Some('0') => out.push('\0'),
                    Some('x') => {
                        let v = self.hex_digits(2)?;
                        out.push(char::from_u32(v).unwrap_or('\u{fffd}'));
                    }
                    Some('u') => {
                        let v = self.hex_digits(4)?;
                        out.push(char::from_u32(v).unwrap_or('\u{fffd}'));
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
                    Some('u') => {
                        let v = self.hex_digits(4)?;
                        cur.push(char::from_u32(v).unwrap_or('\u{fffd}'));
                    }
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
            | Tok::Str(_)
            | Tok::Tpl(_)
            | Tok::Regex { .. }
            | Tok::Kw(Kw::This | Kw::True | Kw::False | Kw::Null)
            | Tok::P(P::RParen | P::RBracket | P::RBrace | P::Inc | P::Dec)
    )
}

fn is_ident_start(c: char) -> bool {
    c == '_' || c == '$' || c.is_alphabetic()
}

fn is_ident_part(c: char) -> bool {
    c == '_' || c == '$' || c.is_alphanumeric()
}

fn keyword(s: &str) -> Option<Kw> {
    use Kw::{
        Break, Case, Catch, Class, Const, Continue, Default, Delete, Do, Else, Extends, False,
        Finally, For, Function, If, In, Instanceof, Let, New, Null, Return, Super, Switch, This,
        Throw, True, Try, Typeof, Unsupported as U, Var, Void, While,
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
        "with" => U("with"),
        "debugger" => U("debugger"),
        "enum" => U("enum"),
        "import" => U("import"),
        "export" => U("export"),
        "yield" => U("yield"),
        _ => return None,
    })
}
