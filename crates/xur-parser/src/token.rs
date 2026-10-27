use std::fmt;

use strum::{Display, EnumString};

use xur_span::Span;

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
    Enum,
    Impl,
    Match,
    True,
    False,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenKind<'src> {
    // --- Literals (zero-copy slices of the source) ---
    /// The raw text between the quotes; escape sequences are not yet resolved.
    StringLit(&'src str),
    /// `b"..."`: the raw text between the quotes, escapes not yet resolved.
    ByteStrLit(&'src str),
    /// The digits as written, including `_` separators. Always non-negative:
    /// a leading `-` is lexed as a separate `Minus`.
    IntLit(&'src str),
    /// The literal as written, e.g. `1_000.5`.
    FloatLit(&'src str),

    // --- Identifiers & keywords ---
    Ident(&'src str),
    Keyword(Keyword),
    /// A lone `_`, used as a wildcard pattern. `_name` is an `Ident`.
    Underscore,

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
    Colon,
    Hash,
    Arrow,
    FatArrow,
    Question,

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
    Pipe,
    PipePipe,

    Eof,
}

impl fmt::Display for TokenKind<'_> {
    /// Shows the token's source text for fixed tokens and a `<placeholder>`
    /// for tokens whose text varies, e.g. in "expected `;`, found <ident>".
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text = match self {
            Self::StringLit(_) => "<string>",
            Self::ByteStrLit(_) => "<bytes>",
            Self::IntLit(_) => "<int>",
            Self::FloatLit(_) => "<float>",
            Self::Ident(_) => "<ident>",
            Self::Keyword(keyword) => return keyword.fmt(f),
            Self::Underscore => "_",
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
            Self::Colon => ":",
            Self::Hash => "#",
            Self::Arrow => "->",
            Self::FatArrow => "=>",
            Self::Question => "?",
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
            Self::Pipe => "|",
            Self::PipePipe => "||",
            Self::Eof => "<eof>",
        };
        f.write_str(text)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Token<'src> {
    pub kind: TokenKind<'src>,
    pub span: Span,
}

impl<'src> Token<'src> {
    pub(crate) fn new(kind: TokenKind<'src>, span: Span) -> Self {
        Self { kind, span }
    }
}
