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
    /// `[T]`, a list of `T`.
    List(Box<Ty>),
    /// `@`, record mode: values are coerced to records, and `T::new()`
    /// creates an empty record.
    Record,
    /// `_`, or no type written at all, as for the closure parameter in
    /// `|x| x + 1`.
    Infer,
    /// `fn(A, B) -> C`, a function type.
    Fn { params: Vec<Ty>, ret: Option<Box<Ty>> },
    /// The type of a `self` parameter written without one; `Self`.
    ImplicitSelf,
    /// A type that failed to parse; the error has already been reported.
    Err,
}
