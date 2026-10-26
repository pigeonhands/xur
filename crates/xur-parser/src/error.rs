use thiserror::Error;

use xur_span::Span;

/// What went wrong while lexing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum LexErrorKind {
    #[error("unexpected character `{0}`")]
    UnexpectedChar(char),
    #[error("unterminated string literal")]
    UnterminatedString,
    #[error("unterminated block comment")]
    UnterminatedBlockComment,
    #[error("integer literal is too large")]
    IntTooLarge,
}

/// A lexing error and the source range it covers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[error("{kind}")]
pub struct LexError {
    pub kind: LexErrorKind,
    pub span: Span,
}
