use xur_span::{Ident, Span};

use crate::{Lit, Path};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pat {
    pub kind: PatKind,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PatKind {
    /// `_`
    Wild,
    /// A binding, `x`. As in rustc, whether a lone identifier actually names
    /// a unit variant or constant is decided during name resolution.
    Ident(Ident),
    /// `0`, `"text"`, `true`
    Lit(Lit),
    /// `MathError::DivideByZero`
    Path(Path),
    /// `Ok(value)`, `Shape::Rect(w, h)`
    TupleStruct(Path, Vec<Pat>),
    /// `(a, b)`; `()` matches the unit value.
    Tuple(Vec<Pat>),
    /// `(pat)`, kept so the tree records exactly what was written.
    Paren(Box<Pat>),
    /// A pattern that failed to parse; the error has already been reported.
    Err,
}
