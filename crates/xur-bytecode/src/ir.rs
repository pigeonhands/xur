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
use xur_resolve::{DefId, ModuleIdx, NativeFnId, NativeTypeId};
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
    Bytes(Arc<[u8]>),
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

/// A type written in the source, as checked at runtime. Xur is checked
/// dynamically: where a type is written it is enforced when the value
/// arrives, and where it isn't (or is generic) anything goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RtType {
    /// Not checked: no type, `_`, or a type parameter.
    Any,
    Unit,
    Bool,
    Str,
    Bytes,
    Float,
    /// An integer type, with the range its values must be in.
    Int {
        name: &'static str,
        min: i64,
        max: i64,
    },
    Tuple(Vec<RtType>),
    /// A list. Only the list itself is checked, not each element, so a
    /// check doesn't take time proportional to the list's length.
    List,
    /// A struct or enum defined in Xur, by its definition.
    Adt {
        def: DefId,
        name: String,
    },
    Native {
        id: NativeTypeId,
        name: String,
    },
}

/// The fields a record is built with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecordFields {
    /// `E::V(a, b)`, or none for `E::V`.
    Positional(Vec<Reg>),
    /// `E::V { x: a, y: b }`, as written.
    Named(Vec<(Symbol, Reg)>),
}

/// A type argument of a call, as in `type_name::<T>()`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TypeArg {
    /// A type known when the code was lowered: its name, and the struct,
    /// enum or native type it is, if any.
    Named { name: String, def: Option<DefId> },
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
    /// `base.field`: a named field of a struct or native value, an element
    /// of a tuple by number, or a module's member.
    Field {
        dst: Reg,
        base: Reg,
        field: Symbol,
    },
    /// A copy of `base` with `field` set to `value`. Structs are values, so
    /// `p.x = 1` stores an updated copy back in `p`.
    SetField {
        dst: Reg,
        base: Reg,
        field: Symbol,
        value: Reg,
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
    /// A record of the native type `ty`, registered from a Rust type's
    /// shape: of its enum variant `variant`, or the struct itself. The VM
    /// checks the fields against the shape.
    Record {
        dst: Reg,
        ty: NativeTypeId,
        variant: Option<Symbol>,
        fields: RecordFields,
    },
    /// Whether `value` is a record of `ty`, and of `variant` if given.
    IsRecord {
        dst: Reg,
        value: Reg,
        ty: NativeTypeId,
        variant: Option<Symbol>,
    },
    /// Whether `value` is a record with exactly `len` fields: a struct
    /// pattern without `..` must name them all.
    RecordArity {
        dst: Reg,
        value: Reg,
        len: u32,
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
    /// Fails at runtime unless `value` has type `ty`.
    CheckType {
        value: Reg,
        ty: RtType,
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

impl std::fmt::Display for RtType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Any => f.write_str("_"),
            Self::Unit => f.write_str("()"),
            Self::Bool => f.write_str("bool"),
            Self::Str => f.write_str("str"),
            Self::Bytes => f.write_str("bytes"),
            Self::Float => f.write_str("float"),
            Self::Int { name, .. } => f.write_str(name),
            Self::Tuple(tys) => {
                let tys: Vec<_> = tys.iter().map(ToString::to_string).collect();
                write!(f, "({})", tys.join(", "))
            }
            Self::List => f.write_str("[_]"),
            Self::Adt { name, .. } | Self::Native { name, .. } => f.write_str(name),
        }
    }
}
