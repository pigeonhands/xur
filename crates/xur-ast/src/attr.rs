use xur_span::Span;

use crate::{Lit, Path};

/// `#[meta]`, attached to the item or field that follows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attribute {
    pub meta: MetaItem,
    pub span: Span,
}

/// The structured contents of an attribute, e.g. `repr(align(4))`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MetaItem {
    pub path: Path,
    pub kind: MetaItemKind,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MetaItemKind {
    /// `repr`
    Word,
    /// `repr(C, align(4))`
    List(Vec<MetaItemInner>),
    /// `offset = 100`
    NameValue(Lit),
}

/// An element of a [`MetaItemKind::List`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MetaItemInner {
    MetaItem(MetaItem),
    Lit(Lit),
}
