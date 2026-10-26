use xur_span::{Span, Symbol};

/// A literal whose value has been decoded from its token.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Lit {
    pub kind: LitKind,
    pub span: Span,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LitKind {
    /// The string's value, with escape sequences resolved.
    Str(Symbol),
    Int(u64),
    /// The digits with `_` separators removed. As in rustc, floats are kept
    /// as text until a type is known, so no precision is lost here.
    Float(Symbol),
    Bool(bool),
    /// A literal that failed to decode; the error has already been reported.
    Err,
}
