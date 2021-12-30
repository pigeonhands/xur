pub(crate) const EOF_CHAR: char = '\0';

#[derive(Clone, Debug, PartialEq)]
pub enum TokenKind {
    Unknown(String),
    Whitespace,
    Identifier(String),
    Symbol(String),
    Numeric(u128),
    String(String),

    Colon,
    At,
    Equals,
    Semicolon,
    Plus,
    Dot,
    Comma,
    OpenParen,
    CloseParen,
    OpenBracket,
    CloseBracket,
}

#[derive(Clone, Debug)]
pub struct Token {
    pub kind: TokenKind,
    pub length: usize,
}
