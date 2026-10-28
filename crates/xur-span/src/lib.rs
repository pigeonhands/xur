//! Source locations and interned names.
//!
//! A [`Span`] is a byte range in the [`SourceMap`]'s global position space.
//! Human-readable line/column positions are computed on demand, so the lexer
//! never has to track them.

mod source_map;
mod span;
mod symbol;

pub use source_map::{
    FileLoader, LineCol, MemoryFileLoader, OverlayFileLoader, RealFileLoader, SourceFile, SourceMap,
};
pub use span::Span;
pub use symbol::{Ident, Symbol};
