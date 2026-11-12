//! VHDL lexer.
//!
//! Produces a stream of tokens that preserves *every* byte of the input —
//! whitespace, newlines, and comments are first-class tokens. This makes
//! the output suitable for round-trip rewriting passes that need to
//! reconstruct the original source with surgical edits.
//!
//! VHDL is case-insensitive, so keyword recognition is performed against
//! a lowercased copy of each identifier. The output `Token` retains the
//! original byte span — the case of the source is never altered here.

use std::ops::Range;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenKind {
    // Trivia (preserved verbatim, never elided).
    Whitespace,   // runs of spaces and tabs (no newline)
    Newline,      // \n or \r\n
    LineComment,  // -- to end of line
    BlockComment, // /* ... */ (VHDL-2008)

    // Atoms.
    Ident,
    /// A reserved word that the formatter tracks by name.
    Keyword(Keyword),
    /// A reserved word the formatter does not need to identify by name,
    /// but which must be distinguished from `Ident` for context-sensitive
    /// rules (notably the `'` disambiguation).
    Reserved,
    String,
    BitString,
    Char,
    Number,

    // Punctuation.
    LParen,
    RParen,
    LBracket,
    RBracket,
    Comma,
    Semi,
    Colon,
    Dot,
    Tick,
    Bar,
    Amp,
    Question,
    At,

    // Operators.
    Assign, // :=
    Arrow,  // =>
    Eq,
    NEq, // /=
    Lt,
    LE, // <=
    Gt,
    GE, // >=
    Plus,
    Minus,
    Star,
    Slash,
    Power, // **
    Box,   // <>
    LL,    // <<
    RR,    // >>

    /// Any byte we don't yet recognize. Preserved verbatim in output —
    /// the formatter never throws bytes away.
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Keyword {
    Architecture,
    Begin,
    Block,
    Case,
    Else,
    Elsif,
    End,
    Entity,
    For,
    Function,
    Generate,
    If,
    Is,
    Loop,
    Package,
    Procedure,
    Process,
    Record,
    Then,
    View,
    When,
    While,
}

/// Classification of a basic identifier against the VHDL reserved word
/// list. Returns:
///   - `Some(Keyword(kw))` for reserved words the formatter tracks by
///     name (these have a dedicated `Keyword` variant),
///   - `Some(Reserved)` for reserved words it does not, and
///   - `None` for ordinary identifiers.
///
/// VHDL is case-insensitive; the comparison is performed against an
/// in-place ASCII-lowercased copy on the stack to avoid allocating on
/// the hot path.
fn classify_ident(s: &str) -> Option<TokenKind> {
    let mut buf = [0u8; 32];
    if s.len() > buf.len() {
        return None;
    }
    for (i, b) in s.bytes().enumerate() {
        buf[i] = b.to_ascii_lowercase();
    }
    let lower = &buf[..s.len()];

    // Reserved words the formatter cares about by name. The full
    // VHDL-2008 reserved word list (the `Reserved` arm below) follows.
    Some(match lower {
        b"architecture" => TokenKind::Keyword(Keyword::Architecture),
        b"begin" => TokenKind::Keyword(Keyword::Begin),
        b"block" => TokenKind::Keyword(Keyword::Block),
        b"case" => TokenKind::Keyword(Keyword::Case),
        b"else" => TokenKind::Keyword(Keyword::Else),
        b"elsif" => TokenKind::Keyword(Keyword::Elsif),
        b"end" => TokenKind::Keyword(Keyword::End),
        b"entity" => TokenKind::Keyword(Keyword::Entity),
        b"for" => TokenKind::Keyword(Keyword::For),
        b"function" => TokenKind::Keyword(Keyword::Function),
        b"generate" => TokenKind::Keyword(Keyword::Generate),
        b"if" => TokenKind::Keyword(Keyword::If),
        b"is" => TokenKind::Keyword(Keyword::Is),
        b"loop" => TokenKind::Keyword(Keyword::Loop),
        b"package" => TokenKind::Keyword(Keyword::Package),
        b"procedure" => TokenKind::Keyword(Keyword::Procedure),
        b"process" => TokenKind::Keyword(Keyword::Process),
        b"record" => TokenKind::Keyword(Keyword::Record),
        b"then" => TokenKind::Keyword(Keyword::Then),
        b"view" => TokenKind::Keyword(Keyword::View),
        b"when" => TokenKind::Keyword(Keyword::When),
        b"while" => TokenKind::Keyword(Keyword::While),

        // Remaining VHDL-2008 reserved words. These are not tracked by
        // name today, but they must lex distinctly from identifiers so
        // that context-sensitive rules (notably `'` disambiguation)
        // behave correctly.
        b"abs"
        | b"access"
        | b"after"
        | b"alias"
        | b"all"
        | b"and"
        | b"array"
        | b"assert"
        | b"assume"
        | b"attribute"
        | b"body"
        | b"buffer"
        | b"bus"
        | b"component"
        | b"configuration"
        | b"constant"
        | b"context"
        | b"cover"
        | b"default"
        | b"disconnect"
        | b"downto"
        | b"exit"
        | b"fairness"
        | b"file"
        | b"force"
        | b"generic"
        | b"group"
        | b"guarded"
        | b"impure"
        | b"in"
        | b"inertial"
        | b"inout"
        | b"label"
        | b"library"
        | b"linkage"
        | b"literal"
        | b"map"
        | b"mod"
        | b"nand"
        | b"new"
        | b"next"
        | b"nor"
        | b"not"
        | b"null"
        | b"of"
        | b"on"
        | b"open"
        | b"or"
        | b"others"
        | b"out"
        | b"parameter"
        | b"port"
        | b"postponed"
        | b"property"
        | b"protected"
        | b"pure"
        | b"range"
        | b"register"
        | b"reject"
        | b"release"
        | b"rem"
        | b"report"
        | b"restrict"
        | b"restrict_guarantee"
        | b"return"
        | b"rol"
        | b"ror"
        | b"select"
        | b"sequence"
        | b"severity"
        | b"shared"
        | b"signal"
        | b"sla"
        | b"sll"
        | b"sra"
        | b"srl"
        | b"strong"
        | b"subtype"
        | b"to"
        | b"transport"
        | b"type"
        | b"unaffected"
        | b"units"
        | b"until"
        | b"use"
        | b"variable"
        | b"vmode"
        | b"vprop"
        | b"vunit"
        | b"wait"
        | b"with"
        | b"xnor"
        | b"xor" => TokenKind::Reserved,

        _ => return None,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Token {
    pub kind: TokenKind,
    pub start: usize,
    pub end: usize,
}

impl Token {
    pub fn span(&self) -> Range<usize> {
        self.start..self.end
    }

    pub fn text<'a>(&self, src: &'a str) -> &'a str {
        &src[self.start..self.end]
    }

    pub fn is_trivia(&self) -> bool {
        matches!(
            self.kind,
            TokenKind::Whitespace
                | TokenKind::Newline
                | TokenKind::LineComment
                | TokenKind::BlockComment
        )
    }
}

pub fn tokenize(src: &str) -> Vec<Token> {
    let mut lexer = Lexer::new(src);
    let mut tokens = Vec::new();
    while let Some(tok) = lexer.next_token() {
        tokens.push(tok);
    }
    tokens
}

struct Lexer<'src> {
    src: &'src str,
    bytes: &'src [u8],
    pos: usize,
    /// The kind of the most recent non-trivia token, used to disambiguate
    /// `'` between a character literal and an attribute tick. See
    /// `lex_tick_or_char` for the rule.
    prev_significant: Option<TokenKind>,
}

impl<'src> Lexer<'src> {
    fn new(src: &'src str) -> Self {
        Self {
            src,
            bytes: src.as_bytes(),
            pos: 0,
            prev_significant: None,
        }
    }

    fn next_token(&mut self) -> Option<Token> {
        if self.pos >= self.bytes.len() {
            return None;
        }
        let start = self.pos;
        let b = self.bytes[self.pos];

        let kind = match b {
            b' ' | b'\t' => self.lex_horizontal_ws(),
            b'\n' | b'\r' => self.lex_newline(),
            b'-' if self.peek(1) == Some(b'-') => self.lex_line_comment(),
            b'/' if self.peek(1) == Some(b'*') => self.lex_block_comment(),
            b'"' => self.lex_string(),
            b'\'' => self.lex_tick_or_char(),
            b'0'..=b'9' => self.lex_number(),
            b if is_ident_start(b) => self.lex_ident_or_bitstring(),
            _ => self.lex_punctuation(),
        };

        let tok = Token {
            kind,
            start,
            end: self.pos,
        };
        if !tok.is_trivia() {
            self.prev_significant = Some(tok.kind);
        }
        Some(tok)
    }

    fn peek(&self, offset: usize) -> Option<u8> {
        self.bytes.get(self.pos + offset).copied()
    }

    fn bump(&mut self) {
        self.pos += 1;
    }

    fn lex_horizontal_ws(&mut self) -> TokenKind {
        while matches!(self.peek(0), Some(b' ' | b'\t')) {
            self.bump();
        }
        TokenKind::Whitespace
    }

    fn lex_newline(&mut self) -> TokenKind {
        if self.peek(0) == Some(b'\r') {
            self.bump();
            if self.peek(0) == Some(b'\n') {
                self.bump();
            }
        } else {
            self.bump();
        }
        TokenKind::Newline
    }

    fn lex_line_comment(&mut self) -> TokenKind {
        // Already at the first '-'.
        self.bump();
        self.bump();
        while let Some(b) = self.peek(0) {
            if b == b'\n' || b == b'\r' {
                break;
            }
            self.bump();
        }
        TokenKind::LineComment
    }

    fn lex_block_comment(&mut self) -> TokenKind {
        // VHDL-2008 /* ... */ — does not nest.
        self.bump(); // /
        self.bump(); // *
        while self.pos < self.bytes.len() {
            if self.peek(0) == Some(b'*') && self.peek(1) == Some(b'/') {
                self.bump();
                self.bump();
                break;
            }
            self.bump();
        }
        TokenKind::BlockComment
    }

    fn lex_string(&mut self) -> TokenKind {
        self.bump(); // opening "
        loop {
            match self.peek(0) {
                None | Some(b'\n') | Some(b'\r') => break, // unterminated; bail
                Some(b'"') => {
                    self.bump();
                    // VHDL escapes embedded quotes by doubling them.
                    if self.peek(0) == Some(b'"') {
                        self.bump();
                        continue;
                    }
                    break;
                }
                Some(_) => self.bump(),
            }
        }
        TokenKind::String
    }

    fn lex_tick_or_char(&mut self) -> TokenKind {
        // The `'` token in VHDL is overloaded: it introduces both
        // character literals (`'a'`) and attribute / qualified-expression
        // ticks (`x'length`, `string'("hi")`).
        //
        // Rule: if the most recent significant token closes an
        // expression — an identifier, `)`, `]`, or any literal — then
        // `'` is a tick. Otherwise we *try* to consume `'<char>'` as a
        // character literal and fall back to a tick if that fails.
        //
        // Note that we deliberately do *not* treat keywords as
        // expression-closing, because constructs like `when '0' =>`
        // place a character literal directly after a keyword.
        let prev_closes_expr = matches!(
            self.prev_significant,
            Some(
                TokenKind::Ident
                    | TokenKind::RParen
                    | TokenKind::RBracket
                    | TokenKind::String
                    | TokenKind::BitString
                    | TokenKind::Char
                    | TokenKind::Number
            )
        );

        if !prev_closes_expr && self.peek(1).is_some() && self.peek(2) == Some(b'\'') {
            self.bump(); // '
            self.bump(); // <char>
            self.bump(); // '
            return TokenKind::Char;
        }

        self.bump();
        TokenKind::Tick
    }

    fn lex_number(&mut self) -> TokenKind {
        // Decimal or based literal. Both share the same integer prefix.
        while matches!(self.peek(0), Some(b) if b.is_ascii_digit() || b == b'_') {
            self.bump();
        }
        if self.peek(0) == Some(b'#') {
            // Based literal: <base> # <body> # [exponent]
            self.bump();
            while let Some(b) = self.peek(0) {
                self.bump();
                if b == b'#' {
                    break;
                }
            }
            self.lex_optional_exponent();
            return TokenKind::Number;
        }
        if self.peek(0) == Some(b'.') && matches!(self.peek(1), Some(b) if b.is_ascii_digit()) {
            self.bump();
            while matches!(self.peek(0), Some(b) if b.is_ascii_digit() || b == b'_') {
                self.bump();
            }
        }
        self.lex_optional_exponent();
        TokenKind::Number
    }

    fn lex_optional_exponent(&mut self) {
        if matches!(self.peek(0), Some(b'e' | b'E')) {
            self.bump();
            if matches!(self.peek(0), Some(b'+' | b'-')) {
                self.bump();
            }
            while matches!(self.peek(0), Some(b) if b.is_ascii_digit()) {
                self.bump();
            }
        }
    }

    fn lex_ident_or_bitstring(&mut self) -> TokenKind {
        let start = self.pos;
        while matches!(self.peek(0), Some(b) if is_ident_continue(b)) {
            self.bump();
        }
        let text = &self.src[start..self.pos];

        // Bit-string literal: a base prefix (B, O, X, D, UB, ...) directly
        // followed by a `"`-delimited body. The optional length prefix
        // (e.g. the `8` in `8X"FF"`) is parsed as a number first, so by
        // the time we land here we have only the letter prefix.
        if self.peek(0) == Some(b'"') && is_bit_string_prefix(text) {
            self.bump(); // opening "
            while let Some(b) = self.peek(0) {
                if b == b'"' {
                    self.bump();
                    break;
                }
                if b == b'\n' || b == b'\r' {
                    break;
                }
                self.bump();
            }
            return TokenKind::BitString;
        }

        classify_ident(text).unwrap_or(TokenKind::Ident)
    }

    fn lex_punctuation(&mut self) -> TokenKind {
        let b = self.bytes[self.pos];
        let next = self.peek(1);

        // Two-byte operators take priority over single-byte ones.
        let two = match (b, next) {
            (b':', Some(b'=')) => Some(TokenKind::Assign),
            (b'=', Some(b'>')) => Some(TokenKind::Arrow),
            (b'/', Some(b'=')) => Some(TokenKind::NEq),
            (b'<', Some(b'=')) => Some(TokenKind::LE),
            (b'>', Some(b'=')) => Some(TokenKind::GE),
            (b'<', Some(b'>')) => Some(TokenKind::Box),
            (b'<', Some(b'<')) => Some(TokenKind::LL),
            (b'>', Some(b'>')) => Some(TokenKind::RR),
            (b'*', Some(b'*')) => Some(TokenKind::Power),
            _ => None,
        };
        if let Some(kind) = two {
            self.bump();
            self.bump();
            return kind;
        }

        let kind = match b {
            b'(' => TokenKind::LParen,
            b')' => TokenKind::RParen,
            b'[' => TokenKind::LBracket,
            b']' => TokenKind::RBracket,
            b',' => TokenKind::Comma,
            b';' => TokenKind::Semi,
            b':' => TokenKind::Colon,
            b'.' => TokenKind::Dot,
            b'|' => TokenKind::Bar,
            b'&' => TokenKind::Amp,
            b'?' => TokenKind::Question,
            b'@' => TokenKind::At,
            b'=' => TokenKind::Eq,
            b'<' => TokenKind::Lt,
            b'>' => TokenKind::Gt,
            b'+' => TokenKind::Plus,
            b'-' => TokenKind::Minus,
            b'*' => TokenKind::Star,
            b'/' => TokenKind::Slash,
            _ => TokenKind::Unknown,
        };
        self.bump();
        kind
    }
}

fn is_ident_start(b: u8) -> bool {
    b.is_ascii_alphabetic()
}

fn is_ident_continue(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

fn is_bit_string_prefix(s: &str) -> bool {
    // VHDL-2008 introduced the unsigned/signed forms (UB, UO, UX, SB,
    // SO, SX). Pre-2008 only allowed B, O, X, D.
    let mut buf = [0u8; 2];
    if s.len() > buf.len() {
        return false;
    }
    for (i, b) in s.bytes().enumerate() {
        buf[i] = b.to_ascii_uppercase();
    }
    matches!(
        &buf[..s.len()],
        b"B" | b"O" | b"X" | b"D" | b"UB" | b"UO" | b"UX" | b"SB" | b"SO" | b"SX"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(src: &str) -> Vec<TokenKind> {
        tokenize(src)
            .into_iter()
            .filter(|t| !t.is_trivia())
            .map(|t| t.kind)
            .collect()
    }

    #[test]
    fn keywords_are_case_insensitive() {
        let ks = kinds("PROCESS process Process pRoCeSs");
        assert_eq!(
            ks,
            vec![
                TokenKind::Keyword(Keyword::Process),
                TokenKind::Keyword(Keyword::Process),
                TokenKind::Keyword(Keyword::Process),
                TokenKind::Keyword(Keyword::Process),
            ]
        );
    }

    #[test]
    fn process_inside_string_is_not_a_keyword() {
        let ks = kinds("\"process\"");
        assert_eq!(ks, vec![TokenKind::String]);
    }

    #[test]
    fn process_inside_line_comment_is_not_a_keyword() {
        let ks = kinds("-- process here\nbegin");
        assert_eq!(ks, vec![TokenKind::Keyword(Keyword::Begin)]);
    }

    #[test]
    fn char_literal_after_arrow() {
        // (others => '0') — the '0' must lex as Char, not as two ticks.
        // `others` is a VHDL reserved word, hence `Reserved`.
        let ks = kinds("(others => '0')");
        assert_eq!(
            ks,
            vec![
                TokenKind::LParen,
                TokenKind::Reserved,
                TokenKind::Arrow,
                TokenKind::Char,
                TokenKind::RParen,
            ]
        );
    }

    #[test]
    fn attribute_tick_after_identifier() {
        // x'length — the ' must lex as Tick, not as the start of a char.
        let ks = kinds("x'length");
        assert_eq!(
            ks,
            vec![TokenKind::Ident, TokenKind::Tick, TokenKind::Ident]
        );
    }

    #[test]
    fn case_when_uses_char_literal() {
        // The `'0'` after a reserved word must lex as Char, not as a
        // tick. This is the discriminating test for the rule that
        // reserved words are *not* expression-closing.
        let ks = kinds("when '0' =>");
        assert_eq!(
            ks,
            vec![
                TokenKind::Keyword(Keyword::When),
                TokenKind::Char,
                TokenKind::Arrow
            ]
        );
    }

    #[test]
    fn bit_string_literal() {
        let ks = kinds("X\"FF\"");
        assert_eq!(ks, vec![TokenKind::BitString]);
    }

    #[test]
    fn based_literal() {
        let ks = kinds("16#FF#");
        assert_eq!(ks, vec![TokenKind::Number]);
    }

    #[test]
    fn assign_and_arrow() {
        let ks = kinds("a := b => c");
        assert_eq!(
            ks,
            vec![
                TokenKind::Ident,
                TokenKind::Assign,
                TokenKind::Ident,
                TokenKind::Arrow,
                TokenKind::Ident,
            ]
        );
    }

    #[test]
    fn line_comment_with_dashes() {
        let ks = kinds("---- four dashes\n");
        assert_eq!(ks, vec![]);
    }

    #[test]
    fn round_trips_every_byte() {
        let src = "library IEEE;\nuse IEEE.STD_LOGIC_1164.ALL;\n\nentity t is end t;\n";
        let toks = tokenize(src);
        let mut rebuilt = String::new();
        for t in &toks {
            rebuilt.push_str(t.text(src));
        }
        assert_eq!(rebuilt, src);
    }
}
