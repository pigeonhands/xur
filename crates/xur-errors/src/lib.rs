//! Compiler diagnostics.
//!
//! Every compiler stage reports problems as a [`Diag`]; the [`Emitter`] turns
//! them into annotated source snippets. This mirrors the split in
//! `rustc_errors` between building a diagnostic and rendering it.

mod diag;
mod emitter;

pub use diag::{Diag, Label, Level};
pub use emitter::Emitter;
