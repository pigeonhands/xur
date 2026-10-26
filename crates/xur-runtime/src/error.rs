use std::fmt;

use xur_errors::Diag;
use xur_span::Span;

/// An error raised while running a program, such as a type mismatch that
/// could only be detected at runtime.
#[derive(Debug, Clone, PartialEq)]
pub struct RuntimeError {
    pub message: String,
    /// Where the error happened, once known. Errors raised inside native
    /// functions get the span of the call that reached them.
    pub span: Option<Span>,
    pub notes: Vec<String>,
}

pub type RunResult<T> = Result<T, RuntimeError>;

impl RuntimeError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            span: None,
            notes: Vec::new(),
        }
    }

    /// Records `span` as where the error happened, unless a more precise
    /// location is already known.
    pub fn at(mut self, span: Span) -> Self {
        self.span.get_or_insert(span);
        self
    }

    pub fn with_note(mut self, note: impl Into<String>) -> Self {
        self.notes.push(note.into());
        self
    }

    /// The error as a diagnostic, located at its span or else `fallback`.
    pub fn into_diag(self, fallback: Span) -> Diag {
        let span = self.span.unwrap_or(fallback);
        let mut diag = Diag::error(self.message, span).with_label(span, "error happened here");
        for note in self.notes {
            diag = diag.with_note(note);
        }
        diag
    }
}

impl fmt::Display for RuntimeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for RuntimeError {}
