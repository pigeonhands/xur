//! Source locations and interned names.
//!
//! A [`Span`] is just a byte range into the source text. Human-readable
//! line/column positions are computed on demand with a [`LineIndex`], so the
//! lexer never has to track them.

mod line_index;
mod span;
mod symbol;

pub use line_index::{LineCol, LineIndex};
pub use span::Span;
pub use symbol::{Ident, Symbol};
