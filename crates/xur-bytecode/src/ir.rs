//! The instruction set.
//!
//! A [`Function`] is a control-flow graph of [`Block`]s over virtual
//! registers. Each block runs its instructions in order and ends in a
//! [`Terminator`] that picks the next block. Registers hold dynamically
//! typed values; nothing here assumes types, so the same code can be
//! interpreted or, later, compiled by a JIT that specialises it.

use std::fmt;
use std::sync::Arc;

use xur_ast::{BinOpKind, UnOp};
use xur_resolve::{DefId, ModuleIdx, NativeFnId};
use xur_span::{Span, Symbol};

/// A virtual register of a function.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Reg(pub u32);

impl fmt::Debug for Reg {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "r{}", self.0)
    }
}

/// A basic block of a function; `BlockId(0)` is the entry.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct BlockId(pub u32);

impl fmt::Debug for BlockId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "bb{}", self.0)
    }
}

#[derive(Debug)]
pub struct Function {
    /// The function's path, e.g. `Shape::area`, or `<closure>`.
    pub name: String,
    /// The module the function is in, for module-relative operations such
    /// as `std::import` of a relative path.
    pub module: ModuleIdx,
    /// For a closure, the number of captured values, passed in the first
    /// registers.
    pub captures: u32,
    /// The number of parameters, passed in the registers after the
    /// captures.
    pub params: u32,
    pub registers: u32,
    /// The function's type parameters, which calls bind by position.
    pub type_params: Vec<Symbol>,
    pub blocks: Vec<Block>,
    /// The bodies of closures created by this function, by index.
    pub closures: Vec<Arc<Function>>,
    pub span: Span,
}

#[derive(Debug)]
pub struct Block {
    pub insts: Vec<Inst>,
    pub term: Terminator,
    /// Where the terminator came from, for its runtime errors.
    pub term_span: Span,
}

#[derive(Debug)]
pub struct Inst {
    pub kind: InstKind,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Constant {
    Unit,
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(Arc<str>),
}

/// A definition used as a value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DefRef {
    /// A function written in Xur.
    Fn(DefId),
    /// A tuple struct's or tuple variant's constructor, as a function.
    Ctor(DefId),
    /// A constant, evaluated the first time it is used.
    Const(DefId),
    Module(DefId),
    Native(NativeFnId),
}

/// A type argument of a call, as in `type_name::<T>()`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TypeArg {
    /// A type known when the code was lowered, by name.
    Named(String),
    /// The caller's type parameter with this index.
    Param(u32),
}

#[derive(Debug)]
pub enum InstKind {
    Const {
        dst: Reg,
        value: Constant,
    },
    Copy {
        dst: Reg,
        src: Reg,
    },
    Def {
        dst: Reg,
        def: DefRef,
    },
    Unary {
        dst: Reg,
        op: UnOp,
        src: Reg,
    },
    /// Every binary operator but the short-circuiting `&&` and `||`, which
    /// are lowered to branches.
    Binary {
        dst: Reg,
        op: BinOpKind,
        lhs: Reg,
        rhs: Reg,
    },
    Call {
        dst: Reg,
        callee: Reg,
        args: Vec<Reg>,
        type_args: Vec<TypeArg>,
    },
    /// `receiver.method(args)`, dispatched on the receiver at runtime.
    MethodCall {
        dst: Reg,
        receiver: Reg,
        method: Symbol,
        args: Vec<Reg>,
    },
    /// `base.field`: a field of a native value, or a module's member.
    Field {
        dst: Reg,
        base: Reg,
        field: Symbol,
    },
    Index {
        dst: Reg,
        base: Reg,
        index: Reg,
    },
    SetIndex {
        base: Reg,
        index: Reg,
        value: Reg,
    },
    Tuple {
        dst: Reg,
        items: Vec<Reg>,
    },
    List {
        dst: Reg,
        items: Vec<Reg>,
    },
    /// An instance of a struct or variant defined in Xur.
    Adt {
        dst: Reg,
        ctor: DefId,
        fields: Vec<Reg>,
    },
    Range {
        dst: Reg,
        start: Option<Reg>,
        end: Option<Reg>,
        inclusive: bool,
    },
    /// A closure running `closures[function]` of the current function,
    /// capturing the values in `captures`.
    Closure {
        dst: Reg,
        function: u32,
        captures: Vec<Reg>,
    },
    /// Whether `value` is an instance of the struct or variant `ctor`.
    IsCtor {
        dst: Reg,
        value: Reg,
        ctor: DefId,
    },
    /// Whether `value` is a tuple of `len` elements.
    IsTuple {
        dst: Reg,
        value: Reg,
        len: u32,
    },
    /// Element `index` of a tuple, or field `index` of a struct or variant.
    Project {
        dst: Reg,
        value: Reg,
        index: u32,
    },
    /// Starts iterating over `iterable`, for a `for` loop.
    IterStart {
        dst: Reg,
        iterable: Reg,
    },
    /// Fails at runtime with `message`; lowered for code that resolution
    /// already reported as broken.
    Error {
        message: String,
    },
}

#[derive(Debug)]
pub enum Terminator {
    Goto(BlockId),
    Branch {
        cond: Reg,
        then: BlockId,
        otherwise: BlockId,
    },
    Return(Reg),
    /// Advances the iterator `iter`: with its next item in `item`, continue
    /// at `body`; when it is exhausted, at `done`.
    Next {
        iter: Reg,
        item: Reg,
        body: BlockId,
        done: BlockId,
    },
    /// No pattern matched the value; a runtime error.
    NoMatch,
    /// Never reached: the end of code after a `return`, `break` or
    /// `continue`.
    Unreachable,
}
