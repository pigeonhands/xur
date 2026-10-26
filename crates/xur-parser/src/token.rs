use std::fmt;

use strum::{Display, EnumString};

use crate::error::LexErrorKind;
use crate::span::Span;

/// All reserved keywords in Xur.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Display, EnumString)]
#[strum(serialize_all = "lowercase")]
pub enum Keyword {
    Use,
    Pub,
    Fn,
    Let,
    Const,
    Return,
    If,
    Else,
    While,
    For,
    In,
    Loop,
    Break,
    Continue,
    Struct,
    True,
    False,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TokenKind<'src> {
    // --- Literals (zero-copy slices of the source) ---
    /// The raw text between the quotes; escape sequences are not yet resolved.
    StringLit(&'src str),
    /// Always non-negative: a leading `-` is lexed as a separate `Minus`.
    IntLit(u64),
    FloatLit(f64),

    // --- Identifiers & keywords ---
    Ident(&'src str),
    Keyword(Keyword),

    // --- Delimiters ---
    LParen,
    RParen,
    LBrace,
    RBrace,
    LBracket,
    RBracket,

    // --- Punctuation ---
    Semi,
    Comma,
    Dot,
    DotDot,
    DotDotEq,
    ColonColon,
    ColonColonLt,
    Colon,
    Hash,
    Arrow,

    // --- Operators ---
    Eq,
    EqEq,
    BangEq,
    Bang,
    Lt,
    LtEq,
    Gt,
    GtEq,
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    AmpAmp,
    PipePipe,

    /// Malformed input. The lexer keeps going after producing one of these.
    Error(LexErrorKind),
    Eof,
}

impl fmt::Display for TokenKind<'_> {
    /// Shows the token's source text for fixed tokens and a `<placeholder>`
    /// for tokens whose text varies, e.g. in "expected `;`, found <ident>".
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text = match self {
            Self::StringLit(_) => "<string>",
            Self::IntLit(_) => "<int>",
            Self::FloatLit(_) => "<float>",
            Self::Ident(_) => "<ident>",
            Self::Keyword(keyword) => return keyword.fmt(f),
            Self::LParen => "(",
            Self::RParen => ")",
            Self::LBrace => "{",
            Self::RBrace => "}",
            Self::LBracket => "[",
            Self::RBracket => "]",
            Self::Semi => ";",
            Self::Comma => ",",
            Self::Dot => ".",
            Self::DotDot => "..",
            Self::DotDotEq => "..=",
            Self::ColonColon => "::",
            Self::ColonColonLt => "::<",
            Self::Colon => ":",
            Self::Hash => "#",
            Self::Arrow => "->",
            Self::Eq => "=",
            Self::EqEq => "==",
            Self::BangEq => "!=",
            Self::Bang => "!",
            Self::Lt => "<",
            Self::LtEq => "<=",
            Self::Gt => ">",
            Self::GtEq => ">=",
            Self::Plus => "+",
            Self::Minus => "-",
            Self::Star => "*",
            Self::Slash => "/",
            Self::Percent => "%",
            Self::AmpAmp => "&&",
            Self::PipePipe => "||",
            Self::Error(_) => "<error>",
            Self::Eof => "<eof>",
        };
        f.write_str(text)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Token<'src> {
    pub kind: TokenKind<'src>,
    pub span: Span,
}

impl<'src> Token<'src> {
    pub(crate) fn new(kind: TokenKind<'src>, span: Span) -> Self {
        Self { kind, span }
    }
}
