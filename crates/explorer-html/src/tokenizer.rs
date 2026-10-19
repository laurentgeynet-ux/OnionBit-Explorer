// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Tokenizer HTML5 WHATWG — entrée incrémentale par chunks (ADR-0012).
//!
//! Modèle streaming : [`Tokenizer::feed`] alimente le buffer,
//! [`Tokenizer::next_token`] émet un token ou `None` quand il
//! manque des données. Toute coupure de chunk — y compris au milieu
//! d'une balise, d'une entité ou d'un mot-clé — est reprise sans
//! perte grâce aux états persistants et au matching incrémental
//! des mots-clés (`doctype`, `public`, `system`, `[CDATA[`).

use crate::entities::{lookup_named, map_numeric};
use crate::token::{Token, TokenAttribute};

/// Taille à partir de laquelle le préfixe consommé du buffer est
/// purgé (borne mémoire, ADR-0011).
const COMPACT_AT: usize = 64 * 1024;

/// Longueur max d'un nom de balise/attribut/doctype avant coupure
/// tolérée (borne contenu hostile — au-delà on tronque).
const MAX_NAME: usize = 1024;

/// Taille max d'une valeur d'attribut ou d'un identifiant de
/// doctype (borne contenu hostile — au-delà on tronque).
const MAX_VALUE: usize = 64 * 1024;

/// Le texte accumulé est émis en un `Token::Character` dès qu'un
/// token non-texte est prêt ou que ce seuil est atteint — jamais
/// au milieu d'une analyse (entité, mot-clé) en attente de données.
const TEXT_FLUSH: usize = 16 * 1024;

/// État de contenu imposé par le tree-builder après une balise
/// rawtext/RCDATA (`<script>`, `<style>`, `<title>`, `<textarea>`…).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContentKind {
    /// RCDATA : texte brut + entités (`<title>`, `<textarea>`).
    Rcdata,
    /// RAWTEXT : texte brut sans entités (`<style>`, `<iframe>`…).
    RawText,
    /// Données de script (`<script>`) — états escaped simplifiés.
    ScriptData,
    /// PLAINTEXT : tout le reste du flux est du texte.
    PlainText,
}

/// États de la machine — sous-ensemble fidèle de la spec couvrant
/// les balises, attributs (quotés ou non), commentaires, DOCTYPE,
/// CDATA, RCDATA/RAWTEXT/script et PLAINTEXT.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Data,
    TagOpen,
    EndTagOpen,
    TagName,
    BeforeAttrName,
    AttrName,
    AfterAttrName,
    BeforeAttrValue,
    AttrValueQuoted(char),
    AttrValueUnquoted,
    AfterAttrValueQuoted,
    SelfClosing,
    MarkupDeclOpen,
    BogusComment,
    CommentStart,
    CommentStartDash,
    Comment,
    CommentEndDash,
    CommentEnd,
    Doctype,
    BeforeDoctypeName,
    DoctypeName,
    AfterDoctypeName,
    BeforeDoctypePublicId,
    DoctypePublicId(char),
    AfterDoctypePublicId,
    BeforeDoctypeSystemId,
    DoctypeSystemId(char),
    AfterDoctypeSystemId,
    BogusDoctype,
    CdataSection,
    CdataEnd1,
    CdataEnd2,
    Rcdata,
    RcdataLessThan,
    RcdataEndTagOpen,
    RcdataEndTagName,
    RawText,
    RawTextLessThan,
    RawTextEndTagOpen,
    RawTextEndTagName,
    ScriptData,
    ScriptLessThan,
    ScriptEndTagOpen,
    ScriptEndTagName,
    PlainText,
}

/// Balise en cours de construction.
#[derive(Debug, Default)]
struct TagBuild {
    name: String,
    is_end: bool,
    self_closing: bool,
    attributes: Vec<TokenAttribute>,
    attr_name: String,
    attr_value: String,
    in_attribute: bool,
}

/// `<!DOCTYPE>` en cours de construction.
#[derive(Debug, Default)]
struct DoctypeBuild {
    name: String,
    public_id: Option<String>,
    system_id: Option<String>,
    current_id: String,
    force_quirks: bool,
}

/// Issue du matching incrémental de mots-clés.
enum Classify {
    Matched(usize),
    Bogus,
    NeedMore,
}

/// Tokenizer HTML5 à entrée incrémentale.
pub struct Tokenizer {
    buf: String,
    pos: usize,
    state: State,
    finished: bool,
    eof_emitted: bool,
    /// Nom de la dernière balise ouvrante — « appropriate end tag »
    /// pour RCDATA/RAWTEXT/script (spec).
    last_start: String,
    /// `<![CDATA[` toléré (contenu étranger signalé par le
    /// tree-builder) ; sinon `<![` ouvre un bogus comment.
    allow_cdata: bool,
    /// Texte en attente (coalescé avant émission).
    pending_text: String,
    /// Token produit mais retenu : le `pending_text` accumulé avant
    /// lui dans le document sort d'abord (ordre du flux).
    held: Option<Token>,
    tag: TagBuild,
    comment: String,
    doctype: DoctypeBuild,
    /// Nom de balise fermante en cours d'accumulation (rawtext etc.).
    temp: String,
}

impl Tokenizer {
    /// Tokenizer vide — alimenter par [`Tokenizer::feed`].
    #[must_use]
    pub fn new() -> Self {
        Self {
            buf: String::new(),
            pos: 0,
            state: State::Data,
            finished: false,
            eof_emitted: false,
            last_start: String::new(),
            allow_cdata: false,
            pending_text: String::new(),
            held: None,
            tag: TagBuild::default(),
            comment: String::new(),
            doctype: DoctypeBuild::default(),
            temp: String::new(),
        }
    }

    /// Tokenizer sur contenu complet — `feed` + `finish` en un.
    #[must_use]
    pub fn on(input: &str) -> Self {
        let mut t = Self::new();
        t.feed(input);
        t.finish();
        t
    }

    /// Alimente le tokenizer (chunk borné, texte déjà décodé et
    /// newlines normalisés — voir [`crate::encoding`]).
    pub fn feed(&mut self, chunk: &str) {
        self.buf.push_str(chunk);
    }

    /// Signale la fin du flux — les états suspendus se résolvent
    /// (commentaire émis, tag partiel abandonné, `Eof`).
    pub fn finish(&mut self) {
        self.finished = true;
    }

    /// `<![CDATA[` autorisé (le tree-builder le signale en contenu
    /// étranger SVG/MathML).
    pub fn set_cdata_allowed(&mut self, allowed: bool) {
        self.allow_cdata = allowed;
    }

    /// Basche l'état de contenu après une balise rawtext/RCDATA —
    /// appelé par le tree-builder (`<script>` →
    /// [`ContentKind::ScriptData`], `<title>` → Rcdata…).
    pub fn set_content_state(&mut self, kind: ContentKind) {
        self.state = match kind {
            ContentKind::Rcdata => State::Rcdata,
            ContentKind::RawText => State::RawText,
            ContentKind::ScriptData => State::ScriptData,
            ContentKind::PlainText => State::PlainText,
        };
    }

    // -----------------------------------------------------------------
    // Primitives d'entrée
    // -----------------------------------------------------------------

    fn peek(&self) -> Option<char> {
        self.buf[self.pos..].chars().next()
    }

    fn peek2(&self) -> (Option<char>, Option<char>) {
        let mut it = self.buf[self.pos..].chars();
        (it.next(), it.next())
    }

    fn advance(&mut self) -> Option<char> {
        let c = self.peek()?;
        self.pos += c.len_utf8();
        self.compact();
        Some(c)
    }

    /// Purge le préfixe consommé quand il dépasse le seuil (borne
    /// mémoire — les données en cours vivent dans les champs, pas
    /// dans le buffer).
    fn compact(&mut self) {
        if self.pos > COMPACT_AT {
            self.buf.drain(..self.pos);
            self.pos = 0;
        }
    }

    fn more_needed(&self) -> bool {
        !self.finished && self.pos >= self.buf.len()
    }

    // -----------------------------------------------------------------
    // Boucle d'émission
    // -----------------------------------------------------------------

    /// Prochain token, ou `None` s'il manque des données (feed la
    /// suite ou `finish()`).
    pub fn next_token(&mut self) -> Option<Token> {
        loop {
            if let Some(tok) = self.held.take() {
                return Some(tok);
            }
            if self.more_needed() {
                if self.pending_text.is_empty() {
                    return None;
                }
                return Some(Token::Character(std::mem::take(&mut self.pending_text)));
            }
            if self.finished && self.pos >= self.buf.len() {
                return self.flush_eof();
            }
            let before = (self.pos, self.state);
            if let Some(tok) = self.step() {
                // Le texte accumulé le précède dans le document :
                // on le purge d'abord (ordre du flux), le token
                // est retenu pour le prochain appel.
                if self.pending_text.is_empty() {
                    return Some(tok);
                }
                self.held = Some(tok);
                return Some(Token::Character(std::mem::take(&mut self.pending_text)));
            }
            if before == (self.pos, self.state) {
                // Aucun progrès : analyse suspendue en attente
                // de données (entité ou mot-clé coupé) — le
                // texte déjà sûr peut sortir.
                if self.pending_text.is_empty() {
                    return None;
                }
                return Some(Token::Character(std::mem::take(&mut self.pending_text)));
            }
            if self.pending_text.len() >= TEXT_FLUSH {
                return Some(Token::Character(std::mem::take(&mut self.pending_text)));
            }
        }
    }

    /// Tous les tokens disponibles jusqu'à `Eof` ou besoin de
    /// données — pratique pour les tests et le contenu complet.
    pub fn run(&mut self) -> Vec<Token> {
        let mut out = Vec::new();
        while let Some(t) = self.next_token() {
            let done = matches!(t, Token::Eof);
            out.push(t);
            if done {
                break;
            }
        }
        out
    }

    /// Résolution des états suspendus à EOF : caractères orphelins
    /// des constructions abandonnées (`<`, `</`, `<!`, `]]`…),
    /// commentaire/doctype en cours, puis `Eof`.
    fn flush_eof(&mut self) -> Option<Token> {
        if self.eof_emitted {
            return None;
        }
        // Réinjecte en texte les marqueurs d'une construction jamais
        // terminée (les balises/attributs partiels, eux, sont perdus
        // — eof-in-tag de la spec).
        match self.state {
            State::TagOpen
            | State::RcdataLessThan
            | State::RawTextLessThan
            | State::ScriptLessThan => {
                self.pending_text.push('<');
                self.state = State::Data;
            }
            State::EndTagOpen
            | State::RcdataEndTagOpen
            | State::RawTextEndTagOpen
            | State::ScriptEndTagOpen => {
                self.pending_text.push_str("</");
                self.state = State::Data;
            }
            State::RcdataEndTagName | State::RawTextEndTagName | State::ScriptEndTagName => {
                self.pending_text.push_str("</");
                let t = std::mem::take(&mut self.temp);
                self.pending_text.push_str(&t);
                self.state = State::Data;
            }
            State::MarkupDeclOpen => {
                self.pending_text.push_str("<!");
                self.state = State::Data;
            }
            State::CdataEnd1 => {
                self.pending_text.push(']');
                self.state = State::Data;
            }
            State::CdataEnd2 => {
                self.pending_text.push_str("]]");
                self.state = State::Data;
            }
            _ => {}
        }
        if !self.pending_text.is_empty() {
            return Some(Token::Character(std::mem::take(&mut self.pending_text)));
        }
        match self.state {
            State::Comment
            | State::CommentEndDash
            | State::CommentEnd
            | State::CommentStart
            | State::CommentStartDash
            | State::BogusComment => {
                self.state = State::Data;
                return Some(Token::Comment(std::mem::take(&mut self.comment)));
            }
            State::Doctype
            | State::BeforeDoctypeName
            | State::DoctypeName
            | State::AfterDoctypeName
            | State::BeforeDoctypePublicId
            | State::DoctypePublicId(_)
            | State::AfterDoctypePublicId
            | State::BeforeDoctypeSystemId
            | State::DoctypeSystemId(_)
            | State::AfterDoctypeSystemId
            | State::BogusDoctype => return Some(self.emit_doctype()),
            _ => {}
        }
        self.eof_emitted = true;
        Some(Token::Eof)
    }

    // -----------------------------------------------------------------
    // Machine à états
    // -----------------------------------------------------------------

    fn step(&mut self) -> Option<Token> {
        match self.state {
            State::Data => self.step_data(),
            State::TagOpen => self.step_tag_open(),
            State::EndTagOpen => self.step_end_tag_open(),
            State::TagName => self.step_tag_name(),
            State::BeforeAttrName => self.step_before_attr_name(),
            State::AttrName => self.step_attr_name(),
            State::AfterAttrName => self.step_after_attr_name(),
            State::BeforeAttrValue => self.step_before_attr_value(),
            State::AttrValueQuoted(q) => self.step_attr_value_quoted(q),
            State::AttrValueUnquoted => self.step_attr_value_unquoted(),
            State::AfterAttrValueQuoted => self.step_after_attr_value_quoted(),
            State::SelfClosing => self.step_self_closing(),
            State::MarkupDeclOpen => self.step_markup_decl_open(),
            State::BogusComment => self.step_bogus_comment(),
            State::CommentStart => self.step_comment_start(),
            State::CommentStartDash => self.step_comment_start_dash(),
            State::Comment => self.step_comment(),
            State::CommentEndDash => self.step_comment_end_dash(),
            State::CommentEnd => self.step_comment_end(),
            State::Doctype => self.step_doctype(),
            State::BeforeDoctypeName => self.step_before_doctype_name(),
            State::DoctypeName => self.step_doctype_name(),
            State::AfterDoctypeName => self.step_after_doctype_name(),
            State::BeforeDoctypePublicId => self.step_before_doctype_id(true),
            State::DoctypePublicId(q) => self.step_doctype_id(true, q),
            State::AfterDoctypePublicId => self.step_after_doctype_public_id(),
            State::BeforeDoctypeSystemId => self.step_before_doctype_id(false),
            State::DoctypeSystemId(q) => self.step_doctype_id(false, q),
            State::AfterDoctypeSystemId => self.step_after_doctype_system_id(),
            State::BogusDoctype => self.step_bogus_doctype(),
            State::CdataSection => self.step_cdata(),
            State::CdataEnd1 | State::CdataEnd2 => self.step_cdata_end(),
            State::Rcdata => self.step_text_like(State::Rcdata),
            State::RcdataLessThan => {
                self.step_text_less_than(State::Rcdata, State::RcdataEndTagOpen)
            }
            State::RcdataEndTagOpen => self.step_text_end_tag_open(State::RcdataEndTagName),
            State::RcdataEndTagName => self.step_text_end_tag_name(State::Rcdata),
            State::RawText => self.step_text_like(State::RawText),
            State::RawTextLessThan => {
                self.step_text_less_than(State::RawText, State::RawTextEndTagOpen)
            }
            State::RawTextEndTagOpen => self.step_text_end_tag_open(State::RawTextEndTagName),
            State::RawTextEndTagName => self.step_text_end_tag_name(State::RawText),
            State::ScriptData => self.step_text_like(State::ScriptData),
            State::ScriptLessThan => {
                self.step_text_less_than(State::ScriptData, State::ScriptEndTagOpen)
            }
            State::ScriptEndTagOpen => self.step_text_end_tag_open(State::ScriptEndTagName),
            State::ScriptEndTagName => self.step_text_end_tag_name(State::ScriptData),
            State::PlainText => self.step_plain_text(),
        }
    }

    // -----------------------------------------------------------------
    // Data et texte
    // -----------------------------------------------------------------

    fn step_data(&mut self) -> Option<Token> {
        match self.peek() {
            Some('<') => {
                self.advance();
                self.state = State::TagOpen;
                None
            }
            Some('&') => {
                match self.charref(false) {
                    CharRef::Need => {}
                    CharRef::Value(v) => self.pending_text.push_str(&v),
                }
                None
            }
            Some(c) => {
                self.advance();
                self.push_text_char(c);
                None
            }
            None => None,
        }
    }

    fn push_text_char(&mut self, c: char) {
        if c == '\0' {
            self.pending_text.push('\u{FFFD}');
        } else {
            self.pending_text.push(c);
        }
    }

    /// Parsing d'une référence de caractère — `pos` pointe le `&`
    /// (non consommé, pour pouvoir attendre la suite d'un chunk).
    fn charref(&mut self, in_attribute: bool) -> CharRef {
        let rem = &self.buf[self.pos..]; // commence par '&'
        let after = &rem[1..];
        match after.chars().next() {
            // `&` seul en fin de buffer : la suite peut compléter.
            None if !self.finished => CharRef::Need,
            Some('#') => self.charref_numeric(),
            Some(c) if c.is_ascii_alphanumeric() => {
                // La suite alphanumérique doit être complète — sinon
                // on ne peut pas trancher le plus-long-match.
                let run_end = after
                    .find(|c: char| !c.is_ascii_alphanumeric())
                    .unwrap_or(after.len());
                if run_end == after.len() && !self.finished {
                    return CharRef::Need;
                }
                if let Some(m) = lookup_named(after, in_attribute) {
                    self.pos += 1 + m.consumed;
                    CharRef::Value(m.value.to_owned())
                } else {
                    self.pos += 1;
                    CharRef::Value("&".to_owned())
                }
            }
            _ => {
                self.pos += 1;
                CharRef::Value("&".to_owned())
            }
        }
    }

    /// Référence numérique — `pos` pointe `&`, `pos+1` = `#`.
    fn charref_numeric(&mut self) -> CharRef {
        let rem = &self.buf[self.pos + 1..]; // commence par '#'
        let mut i = 1; // après '#'
        let hex = matches!(rem[i..].chars().next(), Some('x' | 'X'));
        if hex {
            i += 1;
        }
        let digits_start = i;
        let digit_pred = if hex {
            |c: char| c.is_ascii_hexdigit()
        } else {
            |c: char| c.is_ascii_digit()
        };
        while i < rem.len() && digit_pred(rem[i..].chars().next().unwrap_or('\0')) {
            i += rem[i..].chars().next().unwrap_or('\0').len_utf8();
        }
        if i == rem.len() && !self.finished {
            return CharRef::Need; // les chiffres peuvent continuer
        }
        if i == digits_start {
            // `&#` non suivi de chiffres → `&#` littéral.
            self.pos += 2;
            return CharRef::Value("&#".to_owned());
        }
        let digits = &rem[digits_start..i];
        let has_semi = rem[i..].starts_with(';');
        let cp = u32::from_str_radix(digits, if hex { 16 } else { 10 }).unwrap_or(u32::MAX);
        self.pos += 1 + i + usize::from(has_semi);
        CharRef::Value(map_numeric(cp).to_string())
    }

    // -----------------------------------------------------------------
    // Balises
    // -----------------------------------------------------------------

    fn step_tag_open(&mut self) -> Option<Token> {
        match self.peek() {
            Some('!') => {
                self.advance();
                self.state = State::MarkupDeclOpen;
                None
            }
            Some('/') => {
                self.advance();
                self.state = State::EndTagOpen;
                None
            }
            Some(c) if c.is_ascii_alphabetic() => {
                self.tag = TagBuild::default();
                self.state = State::TagName;
                None
            }
            Some('?') => {
                self.comment.clear();
                self.state = State::BogusComment;
                None
            }
            Some(_) => {
                // `<` littéral (non suivi d'une balise).
                self.pending_text.push('<');
                self.state = State::Data;
                None
            }
            None => {
                self.pending_text.push('<');
                self.state = State::Data;
                None
            }
        }
    }

    fn step_end_tag_open(&mut self) -> Option<Token> {
        match self.peek() {
            Some(c) if c.is_ascii_alphabetic() => {
                self.tag = TagBuild {
                    is_end: true,
                    ..TagBuild::default()
                };
                self.state = State::TagName;
                None
            }
            Some('>') => {
                // `</>` — ignoré par la spec.
                self.advance();
                self.state = State::Data;
                None
            }
            Some(_) => {
                // `</x` non-alpha → bogus comment jusqu'à '>'.
                self.comment.clear();
                self.state = State::BogusComment;
                None
            }
            None => {
                self.pending_text.push_str("</");
                self.state = State::Data;
                None
            }
        }
    }

    fn step_tag_name(&mut self) -> Option<Token> {
        match self.peek() {
            Some(c) if c.is_ascii_whitespace() => {
                self.advance();
                self.state = State::BeforeAttrName;
            }
            Some('/') => {
                self.advance();
                self.state = State::SelfClosing;
            }
            Some('>') => {
                self.advance();
                return Some(self.emit_tag());
            }
            Some(c) => {
                self.advance();
                if self.tag.name.len() < MAX_NAME {
                    self.tag.name.push(c.to_ascii_lowercase());
                }
            }
            None => {}
        }
        None
    }

    fn step_before_attr_name(&mut self) -> Option<Token> {
        match self.peek() {
            Some(c) if c.is_ascii_whitespace() => {
                self.advance();
            }
            Some('/') => {
                self.advance();
                self.state = State::SelfClosing;
            }
            Some('>') => {
                self.advance();
                return Some(self.emit_tag());
            }
            Some(_) => {
                // Nouvel attribut : on termine l'éventuel précédent
                // (`<a x y>` = deux attributs sans valeur).
                self.flush_attribute();
                self.tag.in_attribute = true;
                self.tag.attr_name.clear();
                self.tag.attr_value.clear();
                self.state = State::AttrName;
            }
            None => {}
        }
        None
    }

    fn step_attr_name(&mut self) -> Option<Token> {
        match self.peek() {
            Some('=') => {
                self.advance();
                self.state = State::BeforeAttrValue;
            }
            Some(c) if c.is_ascii_whitespace() || c == '/' || c == '>' => {
                // Nom suivi d'un séparateur : attribut en suspens —
                // un `=` ultérieur lui lierait encore une valeur
                // (`<a x =1>` → x="1").
                self.state = State::AfterAttrName;
            }
            Some(c) => {
                self.advance();
                if self.tag.attr_name.len() < MAX_NAME {
                    self.tag.attr_name.push(c.to_ascii_lowercase());
                }
            }
            None => {}
        }
        None
    }

    fn step_after_attr_name(&mut self) -> Option<Token> {
        match self.peek() {
            Some(c) if c.is_ascii_whitespace() => {
                self.advance();
            }
            Some('=') => {
                // `x = valeur` — lie à l'attribut en suspens.
                self.advance();
                self.tag.attr_value.clear();
                self.state = State::BeforeAttrValue;
            }
            Some('/') => {
                self.advance();
                self.state = State::SelfClosing;
            }
            Some('>') => {
                self.advance();
                return Some(self.emit_tag());
            }
            Some(_) => {
                // Nouvel attribut : termine le précédent en suspens.
                self.flush_attribute();
                self.tag.in_attribute = true;
                self.tag.attr_name.clear();
                self.tag.attr_value.clear();
                self.state = State::AttrName;
            }
            None => {}
        }
        None
    }

    fn step_before_attr_value(&mut self) -> Option<Token> {
        match self.peek() {
            Some(c) if c.is_ascii_whitespace() => {
                self.advance();
            }
            Some(q @ ('"' | '\'')) => {
                self.advance();
                self.tag.attr_value.clear();
                self.state = State::AttrValueQuoted(q);
            }
            Some('>') => {
                // `attr=>` — valeur vide puis émission.
                self.advance();
                self.flush_attribute();
                return Some(self.emit_tag());
            }
            Some(_) => {
                self.tag.attr_value.clear();
                self.state = State::AttrValueUnquoted;
            }
            None => {}
        }
        None
    }

    fn step_attr_value_quoted(&mut self, quote: char) -> Option<Token> {
        match self.peek() {
            Some(c) if c == quote => {
                self.advance();
                self.flush_attribute();
                self.state = State::AfterAttrValueQuoted;
            }
            Some('&') => match self.charref(true) {
                CharRef::Need => {}
                CharRef::Value(v) => self.tag.attr_value.push_str(&v),
            },
            Some(c) => {
                self.advance();
                if self.tag.attr_value.len() < MAX_VALUE {
                    self.tag.attr_value.push(c);
                }
            }
            None => {}
        }
        None
    }

    fn step_attr_value_unquoted(&mut self) -> Option<Token> {
        match self.peek() {
            Some(c) if c.is_ascii_whitespace() => {
                self.advance();
                self.flush_attribute();
                self.state = State::BeforeAttrName;
            }
            Some('&') => match self.charref(true) {
                CharRef::Need => {}
                CharRef::Value(v) => self.tag.attr_value.push_str(&v),
            },
            Some('>') => {
                self.advance();
                self.flush_attribute();
                return Some(self.emit_tag());
            }
            Some(c) => {
                self.advance();
                if self.tag.attr_value.len() < MAX_VALUE {
                    self.tag.attr_value.push(c);
                }
            }
            None => {}
        }
        None
    }

    fn step_after_attr_value_quoted(&mut self) -> Option<Token> {
        match self.peek() {
            Some(c) if c.is_ascii_whitespace() => {
                self.advance();
                self.state = State::BeforeAttrName;
            }
            Some('/') => {
                self.advance();
                self.state = State::SelfClosing;
            }
            Some('>') => {
                self.advance();
                return Some(self.emit_tag());
            }
            Some(_) => {
                // Caractère inattendu après quote — parse error,
                // on retombe en BeforeAttrName sans consommer.
                self.state = State::BeforeAttrName;
            }
            None => {}
        }
        None
    }

    fn step_self_closing(&mut self) -> Option<Token> {
        match self.peek() {
            Some('>') => {
                self.advance();
                self.tag.self_closing = true;
                return Some(self.emit_tag());
            }
            Some(_) => {
                // `/x` sans '>' — parse error, attribut inattendu.
                self.state = State::BeforeAttrName;
            }
            None => {}
        }
        None
    }

    /// Émet la balise construite ; attributs en doublon supprimés
    /// (premier gagné — spec). Termine d'abord l'attribut en suspens.
    fn emit_tag(&mut self) -> Token {
        self.state = State::Data;
        self.flush_attribute();
        let tag = std::mem::take(&mut self.tag);
        let mut attributes: Vec<TokenAttribute> = Vec::with_capacity(tag.attributes.len());
        for a in tag.attributes {
            if !attributes.iter().any(|x: &TokenAttribute| x.name == a.name) {
                attributes.push(a);
            }
        }
        if tag.is_end {
            return Token::EndTag { name: tag.name };
        }
        self.last_start.clone_from(&tag.name);
        Token::StartTag {
            name: tag.name,
            attributes,
            self_closing: tag.self_closing,
        }
    }

    /// Termine l'attribut courant (premier gagné sur doublon).
    fn flush_attribute(&mut self) {
        if !self.tag.in_attribute || self.tag.attr_name.is_empty() {
            self.tag.in_attribute = false;
            return;
        }
        let name = std::mem::take(&mut self.tag.attr_name);
        if self
            .tag
            .attributes
            .iter()
            .all(|a: &TokenAttribute| a.name != name)
        {
            self.tag.attributes.push(TokenAttribute {
                name,
                value: std::mem::take(&mut self.tag.attr_value),
            });
        }
        self.tag.in_attribute = false;
    }

    // -----------------------------------------------------------------
    // `<!` : commentaires, doctype, CDATA, bogus
    // -----------------------------------------------------------------

    fn step_markup_decl_open(&mut self) -> Option<Token> {
        match self.classify(&["--", "doctype", "[cdata["]) {
            Classify::Matched(len) => {
                let raw = self.buf[self.pos..self.pos + len].to_owned();
                self.pos += len;
                match raw.to_ascii_lowercase().as_str() {
                    "--" => {
                        self.comment.clear();
                        self.state = State::CommentStart;
                    }
                    "doctype" => {
                        self.doctype = DoctypeBuild::default();
                        self.state = State::Doctype;
                    }
                    _ => {
                        if self.allow_cdata {
                            self.state = State::CdataSection;
                        } else {
                            // `<![CDATA[` hors contenu étranger =
                            // bogus comment incluant le marqueur.
                            self.comment.clear();
                            self.comment.push_str(&raw);
                            self.state = State::BogusComment;
                        }
                    }
                }
                None
            }
            Classify::Bogus => {
                self.comment.clear();
                self.state = State::BogusComment;
                None
            }
            Classify::NeedMore => None,
        }
    }

    /// Matching incrémental de mots-clés (case-insensitive) —
    /// `Matched(len)` = consommé, `NeedMore` = trancher plus tard.
    fn classify(&self, kws: &[&'static str]) -> Classify {
        let rem = &self.buf[self.pos..];
        let mut alive: Vec<&'static str> = kws.to_vec();
        for (i, ch) in rem.chars().enumerate() {
            alive.retain(|kw| {
                kw.as_bytes()
                    .get(i)
                    .is_some_and(|&b| ch.eq_ignore_ascii_case(&(b as char)))
            });
            if alive.len() == 1 && alive[0].len() == i + 1 {
                return Classify::Matched(alive[0].len());
            }
            if alive.is_empty() {
                return Classify::Bogus;
            }
        }
        // Buffer épuisé : un candidat exactement couvert gagne.
        if let Some(kw) = alive.iter().find(|kw| kw.len() == rem.len()) {
            return Classify::Matched(kw.len());
        }
        if self.finished {
            return Classify::Bogus;
        }
        Classify::NeedMore
    }

    fn step_bogus_comment(&mut self) -> Option<Token> {
        match self.peek() {
            Some('>') => {
                self.advance();
                self.state = State::Data;
                Some(Token::Comment(std::mem::take(&mut self.comment)))
            }
            Some(c) => {
                self.advance();
                if self.comment.len() < MAX_VALUE {
                    self.comment.push(if c == '\0' { '\u{FFFD}' } else { c });
                }
                None
            }
            None => None,
        }
    }

    fn step_comment_start(&mut self) -> Option<Token> {
        match self.peek() {
            Some('-') => {
                self.advance();
                self.state = State::CommentStartDash;
                None
            }
            Some('>') => {
                // `<!-->` — commentaire vide.
                self.advance();
                self.state = State::Data;
                Some(Token::Comment(String::new()))
            }
            Some(c) => {
                self.comment.push(c);
                self.advance();
                self.state = State::Comment;
                None
            }
            None => None,
        }
    }

    fn step_comment_start_dash(&mut self) -> Option<Token> {
        match self.peek() {
            Some('-') => {
                self.advance();
                self.state = State::CommentEnd;
                None
            }
            Some('>') => {
                // `<!--->` — commentaire vide.
                self.advance();
                self.state = State::Data;
                Some(Token::Comment(String::new()))
            }
            Some(c) => {
                self.comment.push('-');
                self.comment.push(c);
                self.advance();
                self.state = State::Comment;
                None
            }
            None => None,
        }
    }

    fn step_comment(&mut self) -> Option<Token> {
        match self.peek() {
            Some('-') => {
                self.advance();
                self.state = State::CommentEndDash;
            }
            Some(c) => {
                self.advance();
                if self.comment.len() < MAX_VALUE {
                    self.comment.push(if c == '\0' { '\u{FFFD}' } else { c });
                }
            }
            None => {}
        }
        None
    }

    fn step_comment_end_dash(&mut self) -> Option<Token> {
        match self.peek() {
            Some('-') => {
                self.advance();
                self.state = State::CommentEnd;
                None
            }
            Some(c) => {
                self.comment.push('-');
                self.comment.push(c);
                self.advance();
                self.state = State::Comment;
                None
            }
            None => None,
        }
    }

    fn step_comment_end(&mut self) -> Option<Token> {
        match self.peek() {
            Some('>') => {
                self.advance();
                self.state = State::Data;
                Some(Token::Comment(std::mem::take(&mut self.comment)))
            }
            Some('-') => {
                self.comment.push('-');
                self.advance();
                None
            }
            Some(c) => {
                // `--x` n'est pas une fin : le '--' fait partie du
                // commentaire (approximation de la spec).
                self.comment.push('-');
                self.comment.push('-');
                self.comment.push(c);
                self.advance();
                self.state = State::Comment;
                None
            }
            None => None,
        }
    }

    // -----------------------------------------------------------------
    // DOCTYPE
    // -----------------------------------------------------------------

    fn step_doctype(&mut self) -> Option<Token> {
        match self.peek() {
            Some(c) if c.is_ascii_whitespace() => {
                self.advance();
                self.state = State::BeforeDoctypeName;
                None
            }
            Some('>') => {
                // `<!DOCTYPE>` sans nom → quirks.
                self.advance();
                self.doctype.force_quirks = true;
                Some(self.emit_doctype())
            }
            Some(_) => self.step_before_doctype_name(),
            None => None,
        }
    }

    fn step_before_doctype_name(&mut self) -> Option<Token> {
        match self.peek() {
            Some(c) if c.is_ascii_whitespace() => {
                self.advance();
            }
            Some('>') => {
                self.advance();
                self.doctype.force_quirks = true;
                return Some(self.emit_doctype());
            }
            Some(c) => {
                self.advance();
                if self.doctype.name.len() < MAX_NAME {
                    self.doctype.name.push(c.to_ascii_lowercase());
                }
                self.state = State::DoctypeName;
            }
            None => {}
        }
        None
    }

    fn step_doctype_name(&mut self) -> Option<Token> {
        match self.peek() {
            Some(c) if c.is_ascii_whitespace() => {
                self.advance();
                self.state = State::AfterDoctypeName;
            }
            Some('>') => {
                self.advance();
                return Some(self.emit_doctype());
            }
            Some(c) => {
                self.advance();
                if self.doctype.name.len() < MAX_NAME {
                    self.doctype.name.push(c.to_ascii_lowercase());
                }
            }
            None => {}
        }
        None
    }

    fn step_after_doctype_name(&mut self) -> Option<Token> {
        match self.peek() {
            Some(c) if c.is_ascii_whitespace() => {
                self.advance();
            }
            Some('>') => {
                self.advance();
                return Some(self.emit_doctype());
            }
            Some(_) => match self.classify(&["public", "system"]) {
                Classify::Matched(6) => {
                    self.pos += 6;
                    self.state = State::BeforeDoctypePublicId;
                }
                Classify::Matched(len) => {
                    self.pos += len;
                    self.state = State::BeforeDoctypeSystemId;
                }
                Classify::Bogus => {
                    self.doctype.force_quirks = true;
                    self.state = State::BogusDoctype;
                }
                Classify::NeedMore => {}
            },
            None => {}
        }
        None
    }

    /// `is_public` : `true` = identifiant public, `false` = système.
    fn step_before_doctype_id(&mut self, is_public: bool) -> Option<Token> {
        match self.peek() {
            Some(c) if c.is_ascii_whitespace() => {
                self.advance();
            }
            Some(q @ ('"' | '\'')) => {
                self.advance();
                self.doctype.current_id.clear();
                self.state = if is_public {
                    State::DoctypePublicId(q)
                } else {
                    State::DoctypeSystemId(q)
                };
            }
            Some('>') => {
                // `PUBLIC`/`SYSTEM` sans identifiant → quirks.
                self.advance();
                self.doctype.force_quirks = true;
                return Some(self.emit_doctype());
            }
            Some(_) => {
                self.doctype.force_quirks = true;
                self.state = State::BogusDoctype;
            }
            None => {}
        }
        None
    }

    fn step_doctype_id(&mut self, is_public: bool, quote: char) -> Option<Token> {
        match self.peek() {
            Some(c) if c == quote => {
                self.advance();
                let id = std::mem::take(&mut self.doctype.current_id);
                if is_public {
                    self.doctype.public_id = Some(id);
                    self.state = State::AfterDoctypePublicId;
                } else {
                    self.doctype.system_id = Some(id);
                    self.state = State::AfterDoctypeSystemId;
                }
            }
            Some('>') => {
                // Fin abrupte → quirks.
                self.advance();
                self.doctype.force_quirks = true;
                return Some(self.emit_doctype());
            }
            Some(c) => {
                self.advance();
                if self.doctype.current_id.len() < MAX_VALUE {
                    self.doctype.current_id.push(c);
                }
            }
            None => {}
        }
        None
    }

    fn step_after_doctype_public_id(&mut self) -> Option<Token> {
        match self.peek() {
            Some(c) if c.is_ascii_whitespace() => {
                self.advance();
            }
            Some('>') => {
                self.advance();
                return Some(self.emit_doctype());
            }
            Some(q @ ('"' | '\'')) => {
                // Id système directement après public (sans mot-clé).
                self.advance();
                self.doctype.current_id.clear();
                self.state = State::DoctypeSystemId(q);
            }
            Some(_) => {
                self.state = State::BeforeDoctypeSystemId;
            }
            None => {}
        }
        None
    }

    fn step_after_doctype_system_id(&mut self) -> Option<Token> {
        match self.peek() {
            Some(c) if c.is_ascii_whitespace() => {
                self.advance();
            }
            Some('>') => {
                self.advance();
                return Some(self.emit_doctype());
            }
            Some(_) => {
                self.state = State::BogusDoctype;
            }
            None => {}
        }
        None
    }

    fn step_bogus_doctype(&mut self) -> Option<Token> {
        match self.peek() {
            Some('>') => {
                self.advance();
                return Some(self.emit_doctype());
            }
            Some(_) => {
                self.advance();
            }
            None => {}
        }
        None
    }

    fn emit_doctype(&mut self) -> Token {
        self.state = State::Data;
        let d = std::mem::take(&mut self.doctype);
        Token::Doctype {
            name: if d.name.is_empty() {
                None
            } else {
                Some(d.name)
            },
            public_id: d.public_id,
            system_id: d.system_id,
            force_quirks: d.force_quirks,
        }
    }

    // -----------------------------------------------------------------
    // CDATA (contenu étranger uniquement — `allow_cdata`)
    // -----------------------------------------------------------------

    fn step_cdata(&mut self) -> Option<Token> {
        match self.peek2() {
            (Some(']'), Some(']')) => {
                self.advance();
                self.state = State::CdataEnd1;
            }
            (Some(c), _) => {
                self.advance();
                self.push_text_char(c);
            }
            _ => {}
        }
        None
    }

    fn step_cdata_end(&mut self) -> Option<Token> {
        // `]]>` ferme ; tout autre caractère réinjecte les `]`
        // accumulés dans le texte.
        match self.peek() {
            Some('>') if self.state == State::CdataEnd2 => {
                self.advance();
                self.state = State::Data;
                // Le contenu CDATA forme son propre segment de
                // texte — émis à la fermeture, séparé de la suite.
                if !self.pending_text.is_empty() {
                    return Some(Token::Character(std::mem::take(&mut self.pending_text)));
                }
            }
            Some(']') if self.state == State::CdataEnd1 => {
                self.advance();
                self.state = State::CdataEnd2;
            }
            Some(c) => {
                let brackets = if self.state == State::CdataEnd2 { 2 } else { 1 };
                for _ in 0..brackets {
                    self.pending_text.push(']');
                }
                self.advance();
                self.push_text_char(c);
                self.state = State::CdataSection;
            }
            None => {}
        }
        None
    }

    // -----------------------------------------------------------------
    // RCDATA / RAWTEXT / script / plaintext — factorisés
    // -----------------------------------------------------------------

    /// Corps de texte-like : accumule jusqu'à `<` potentiellement
    /// terminal ; RCDATA résout en plus les entités.
    fn step_text_like(&mut self, state: State) -> Option<Token> {
        let lt_state = match state {
            State::Rcdata => State::RcdataLessThan,
            State::RawText => State::RawTextLessThan,
            State::ScriptData => State::ScriptLessThan,
            _ => state,
        };
        match self.peek() {
            Some('<') => {
                self.advance();
                self.state = lt_state;
            }
            Some('&') if state == State::Rcdata => {
                if let CharRef::Value(v) = self.charref(false) {
                    self.pending_text.push_str(&v);
                }
            }
            Some(c) => {
                self.advance();
                self.push_text_char(c);
            }
            None => {}
        }
        None
    }

    fn step_text_less_than(&mut self, back: State, end_tag_open: State) -> Option<Token> {
        if self.peek() == Some('/') {
            self.advance();
            self.temp.clear();
            self.state = end_tag_open;
        } else {
            // `<` non terminal : littéral (spec — y compris EOF).
            self.pending_text.push('<');
            self.state = back;
        }
        None
    }

    fn step_text_end_tag_open(&mut self, end_tag_name: State) -> Option<Token> {
        match self.peek() {
            Some(c) if c.is_ascii_alphabetic() => {
                self.state = end_tag_name;
            }
            Some(_) => {
                self.pending_text.push_str("</");
                self.state = match end_tag_name {
                    State::RcdataEndTagName => State::Rcdata,
                    State::RawTextEndTagName => State::RawText,
                    _ => State::ScriptData,
                };
            }
            None => {}
        }
        None
    }

    /// Accumule le nom de la balise fermante ; si c'est l'«
    /// appropriate end tag » (== dernière ouvrante), devient un
    /// vrai `EndTag` — sinon le `</nom` était du texte littéral.
    fn step_text_end_tag_name(&mut self, back: State) -> Option<Token> {
        match self.peek() {
            Some(c) if c.is_ascii_alphabetic() => {
                self.advance();
                self.temp.push(c.to_ascii_lowercase());
            }
            Some(c) if c.is_ascii_whitespace() || c == '/' || c == '>' => {
                if self.temp == self.last_start {
                    self.tag = TagBuild {
                        is_end: true,
                        name: std::mem::take(&mut self.temp),
                        ..TagBuild::default()
                    };
                    self.state = State::TagName;
                } else {
                    self.pending_text.push_str("</");
                    self.pending_text.push_str(&self.temp);
                    self.temp.clear();
                    self.state = back;
                }
            }
            Some(c) => {
                // Nom cassé par un caractère non-alpha : texte littéral.
                self.pending_text.push_str("</");
                self.pending_text.push_str(&self.temp);
                self.temp.clear();
                self.pending_text.push(c);
                self.advance();
                self.state = back;
            }
            None => {}
        }
        None
    }

    fn step_plain_text(&mut self) -> Option<Token> {
        if let Some(c) = self.peek() {
            self.advance();
            self.push_text_char(c);
        }
        None
    }
}

impl Default for Tokenizer {
    fn default() -> Self {
        Self::new()
    }
}

/// Issue d'un parsing de référence de caractère.
enum CharRef {
    /// Plus de données nécessaires pour trancher.
    Need,
    /// Valeur résolue (entité ou `&`/`&#` littéral).
    Value(String),
}
