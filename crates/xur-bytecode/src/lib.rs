//! Xur's bytecode, and lowering resolved syntax to it.
//!
//! Each function becomes a control-flow graph of basic blocks over virtual
//! registers (see [`ir`]). The shape is close to Cranelift's IR, so a JIT can
//! later compile the same code the interpreter runs.

mod display;
pub mod ir;
mod items;
mod lower;
mod types;

pub use display::display_function;
pub use ir::{
    Block, BlockId, Constant, DefRef, Function, Inst, InstKind, RecordFields, Reg, RtType,
    Terminator, TypeArg,
};
pub use items::{ItemIndex, ItemRef};
pub use lower::lower;
pub use types::{type_def, type_name};

#[cfg(test)]
mod tests;
