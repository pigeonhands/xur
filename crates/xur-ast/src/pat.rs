use xur_span::{Ident, Span};

use crate::{Lit, NodeId, Path};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pat {
    pub id: NodeId,
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
    /// `Point { x, y: 0 }`, or `Point { x, .. }` with `rest` set to ignore
    /// the fields not named.
    Struct {
        path: Path,
        fields: Vec<PatField>,
        rest: bool,
    },
    /// `(a, b)`; `()` matches the unit value.
    Tuple(Vec<Pat>),
    /// `[a, b]` or `[first, ..]` with `rest` true.
    List(Vec<Pat>, bool),
    /// `(pat)`, kept so the tree records exactly what was written.
    Paren(Box<Pat>),
    /// A pattern that failed to parse; the error has already been reported.
    Err,
}

/// `name: pat` in a struct pattern. In the shorthand `name`, `pat` binds a
/// variable of the same name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PatField {
    pub ident: Ident,
    pub pat: Pat,
    pub shorthand: bool,
    pub span: Span,
}
