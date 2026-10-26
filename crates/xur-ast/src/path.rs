use xur_span::{Ident, Span};

use crate::Ty;

/// `std::io::println`, `Result<u32, MathError>`, `type_name::<T>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Path {
    pub segments: Vec<PathSegment>,
    pub span: Span,
}

impl Path {
    pub fn from_ident(ident: Ident) -> Self {
        Self {
            segments: vec![PathSegment { ident, args: None }],
            span: ident.span,
        }
    }

    /// The identifier if this path is a single segment without generic args.
    pub fn as_ident(&self) -> Option<Ident> {
        match self.segments.as_slice() {
            [PathSegment { ident, args: None }] => Some(*ident),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathSegment {
    pub ident: Ident,
    pub args: Option<GenericArgs>,
}

/// `<u32, MathError>`
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GenericArgs {
    pub args: Vec<Ty>,
    pub span: Span,
}
