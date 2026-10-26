use xur_span::Span;

use crate::Path;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ty {
    pub kind: TyKind,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TyKind {
    /// `u32`, `Result<u32, MathError>`, `math::MathError`
    Path(Path),
    /// `(A, B)`; `()` is the unit type.
    Tuple(Vec<Ty>),
    /// `(T)`, kept so the tree records exactly what was written.
    Paren(Box<Ty>),
    /// `_`, or no type written at all, as for the closure parameter in
    /// `|x| x + 1`.
    Infer,
    /// A type that failed to parse; the error has already been reported.
    Err,
}
