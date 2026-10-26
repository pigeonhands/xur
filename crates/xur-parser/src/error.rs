use thiserror::Error;

use xur_errors::Diag;
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
}

/// A lexing error and the source range it covers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[error("{kind}")]
pub struct LexError {
    pub kind: LexErrorKind,
    pub span: Span,
}

impl From<LexError> for Diag {
    fn from(error: LexError) -> Self {
        let label = match error.kind {
            LexErrorKind::UnexpectedChar(_) => "not valid in Xur source",
            LexErrorKind::UnterminatedString => "missing closing `\"`",
            LexErrorKind::UnterminatedBlockComment => "comment is never closed",
        };
        Diag::error(error.kind.to_string(), error.span).with_label(error.span, label)
    }
}
