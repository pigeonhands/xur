use strum::{Display, EnumString};

#[derive(Debug, Clone, PartialEq)]
pub struct Span {
    /// Byte offset of the first character of this token.
    pub start: usize,
    /// Byte offset one past the last character of this token.
    pub end: usize,
    /// 1-based line number.
    pub line: u32,
    /// 1-based column number (byte-indexed).
    pub col: u32,
}

impl Span {
    pub(crate) fn new(start: usize, end: usize, line: u32, col: u32) -> Self {
        Self { start, end, line, col }
    }

    /// Returns the slice of the original source code corresponding to this span.
    pub fn slice<'src>(&self, src: &'src str) -> &'src str {
        &src[self.start..self.end]
    }
}

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

#[derive(Debug, Clone, PartialEq, Display)]
pub enum TokenKind<'src> {
    // --- Literals (zero-allocation borrowed slices) ---
    #[strum(to_string = "<string>")]
    StringLit(&'src str),
    #[strum(to_string = "<int>")]
    IntLit(i64),
    #[strum(to_string = "<float>")]
    FloatLit(f64),

    // --- Identifiers & keywords ---
    #[strum(to_string = "<ident>")]
    Ident(&'src str),
    /// A reserved keyword — display delegates to `Keyword`'s own `Display`.
    #[strum(to_string = "{0}")]
    Keyword(Keyword),

    // --- Punctuation ---
    #[strum(to_string = "(")]
    LParen,
    #[strum(to_string = ")")]
    RParen,
    #[strum(to_string = "{")]
    LBrace,
    #[strum(to_string = "}}")]
    RBrace,
    #[strum(to_string = "[")]
    LBracket,
    #[strum(to_string = "]")]
    RBracket,
    #[strum(to_string = ";")]
    Semi,
    #[strum(to_string = ",")]
    Comma,
    #[strum(to_string = ".")]
    Dot,
    #[strum(to_string = "..")]
    DotDot,
    #[strum(to_string = "..=")]
    DotDotEq,
    #[strum(to_string = "::")]
    ColonColon,
    #[strum(to_string = "::<")]
    ColonColonLt,
    #[strum(to_string = ":")]
    Colon,
    #[strum(to_string = "#")]
    Hash,
    #[strum(to_string = "->")]
    Arrow,
    #[strum(to_string = "=")]
    Eq,
    #[strum(to_string = "==")]
    EqEq,
    #[strum(to_string = "!=")]
    BangEq,
    #[strum(to_string = "!")]
    Bang,
    #[strum(to_string = "<")]
    Lt,
    #[strum(to_string = "<=")]
    LtEq,
    #[strum(to_string = ">")]
    Gt,
    #[strum(to_string = ">=")]
    GtEq,
    #[strum(to_string = "+")]
    Plus,
    #[strum(to_string = "-")]
    Minus,
    #[strum(to_string = "*")]
    Star,
    #[strum(to_string = "/")]
    Slash,
    #[strum(to_string = "%")]
    Percent,
    #[strum(to_string = "&&")]
    AmpAmp,
    #[strum(to_string = "||")]
    PipePipe,

    // --- Resilient error tokens ---
    #[strum(to_string = "<error: unterminated string>")]
    UnterminatedString,
    #[strum(to_string = "<unknown>")]
    Unknown(char),

    // --- Meta ---
    #[strum(to_string = "<eof>")]
    Eof,
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
