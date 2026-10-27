//! A JIT compiler for Xur bytecode, using Cranelift. **Not implemented
//! yet**: [`Jit::compile`] always reports that it can't compile a function,
//! and the interpreter runs everything.
//!
//! The bytecode was designed for this. A [`Function`] is already a
//! control-flow graph of basic blocks over virtual registers, which is how
//! Cranelift's IR is shaped, so compiling one is mostly a translation:
//!
//! - each bytecode block becomes a Cranelift block, and each register a
//!   Cranelift variable (`cranelift_frontend` builds the SSA form);
//! - registers hold dynamically typed values, so compiled code works on a
//!   tagged representation and calls back into the runtime for anything
//!   but the common cases (integer and float arithmetic, comparisons,
//!   branches, calls between compiled functions);
//! - the `check` instructions that enforce written types are what tell the
//!   compiler a register's type, so code after them can be specialised.
//!
//! The VM would compile functions once they are called often enough and
//! fall back to interpreting whatever the JIT doesn't support.

use cranelift_jit::{JITBuilder, JITModule};
use thiserror::Error;
use xur_bytecode::Function;

#[derive(Debug, Error)]
pub enum JitError {
    #[error("the JIT can't compile `{0}` yet")]
    Unsupported(String),
    #[error("couldn't set up the JIT for this machine: {0}")]
    Setup(String),
}

/// Machine code for one function. Nothing is compiled yet, so none exist.
pub struct CompiledFn {
    _private: (),
}

/// Compiles bytecode functions to machine code for the host.
pub struct Jit {
    _module: JITModule,
}

impl Jit {
    /// Prepares to compile for the machine this is running on.
    pub fn new() -> Result<Self, JitError> {
        let builder = JITBuilder::new(cranelift_module::default_libcall_names())
            .map_err(|error| JitError::Setup(error.to_string()))?;
        Ok(Self {
            _module: JITModule::new(builder),
        })
    }

    /// Compiles `function` to machine code. Not implemented yet.
    pub fn compile(&mut self, function: &Function) -> Result<CompiledFn, JitError> {
        Err(JitError::Unsupported(function.name.clone()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sets_up_for_the_host_but_compiles_nothing_yet() {
        let mut jit = Jit::new().expect("the host is supported by Cranelift");
        let function = Function {
            name: "f".to_owned(),
            module: xur_resolve::ModuleIdx(0),
            captures: 0,
            params: 0,
            registers: 0,
            type_params: Vec::new(),
            blocks: Vec::new(),
            closures: Vec::new(),
            span: xur_span::Span::default(),
        };
        assert_eq!(
            jit.compile(&function).err().map(|e| e.to_string()),
            Some("the JIT can't compile `f` yet".to_owned())
        );
    }
}
