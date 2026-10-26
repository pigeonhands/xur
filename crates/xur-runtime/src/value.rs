//! Runtime values.
//!
//! Xur is dynamically checked, so every value carries what it is. Lists are
//! shared and mutable, like Python's; everything else is immutable, so
//! cloning a value is always cheap.

use std::cell::RefCell;
use std::rc::Rc;

use xur_resolve::{DefId, NativeFnId, NativeTypeId};
use xur_span::Symbol;

#[derive(Debug, Clone)]
pub enum Value {
    /// `()`, the value of expressions that produce nothing.
    Unit,
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(Rc<str>),
    Tuple(Rc<[Value]>),
    List(Rc<RefCell<Vec<Value>>>),
    /// An instance of a struct or enum variant defined in Xur.
    Adt(Rc<Adt>),
    /// An instance of a type implemented in Rust, as named fields.
    Record(Rc<Record>),
    /// A function defined in Xur, including those in `impl` blocks.
    Fn(DefId),
    /// The constructor of a tuple struct or tuple variant used as a
    /// function, as in `Some(1)` or `list.map(Some)`.
    Ctor(DefId),
    /// A function implemented in Rust.
    NativeFn(NativeFnId),
    /// A module: modules are values.
    Module(DefId),
    /// `a..b`, `a..=b`, `a..`, `..b` over integers.
    Range(Range),
}

/// An instance of a struct, or of an enum variant, defined in Xur.
#[derive(Debug, Clone)]
pub struct Adt {
    /// The struct or variant.
    pub ctor: DefId,
    pub fields: Vec<Value>,
}

/// An instance of a Rust type, converted to named fields.
#[derive(Debug, Clone)]
pub struct Record {
    /// The registered native type, if the Rust type was registered; its
    /// methods can then be called on the record.
    pub ty: Option<NativeTypeId>,
    pub fields: Vec<(Symbol, Value)>,
}

impl Record {
    pub fn field(&self, name: Symbol) -> Option<&Value> {
        self.fields
            .iter()
            .find(|(field, _)| *field == name)
            .map(|(_, value)| value)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Range {
    pub start: Option<i64>,
    pub end: Option<i64>,
    /// Whether `end` is included, as in `a..=b`.
    pub inclusive: bool,
}

impl Value {
    pub fn str(s: &str) -> Self {
        Self::Str(s.into())
    }

    pub fn list(items: Vec<Value>) -> Self {
        Self::List(Rc::new(RefCell::new(items)))
    }

    pub fn tuple(items: Vec<Value>) -> Self {
        Self::Tuple(items.into())
    }

    pub fn adt(ctor: DefId, fields: Vec<Value>) -> Self {
        Self::Adt(Rc::new(Adt { ctor, fields }))
    }

    /// How the kind of value is named in error messages.
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Unit => "unit",
            Self::Bool(_) => "bool",
            Self::Int(_) => "integer",
            Self::Float(_) => "float",
            Self::Str(_) => "string",
            Self::Tuple(_) => "tuple",
            Self::List(_) => "list",
            Self::Adt(_) => "struct or enum",
            Self::Record(_) => "native value",
            Self::Fn(_) | Self::NativeFn(_) => "function",
            Self::Ctor(_) => "constructor",
            Self::Module(_) => "module",
            Self::Range(_) => "range",
        }
    }
}

/// Structural equality, as `==` sees it. Values of different kinds are
/// never equal; in particular `1 == 1.0` is false.
impl PartialEq for Value {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Unit, Self::Unit) => true,
            (Self::Bool(a), Self::Bool(b)) => a == b,
            (Self::Int(a), Self::Int(b)) => a == b,
            (Self::Float(a), Self::Float(b)) => a == b,
            (Self::Str(a), Self::Str(b)) => a == b,
            (Self::Tuple(a), Self::Tuple(b)) => a == b,
            (Self::List(a), Self::List(b)) => Rc::ptr_eq(a, b) || *a.borrow() == *b.borrow(),
            (Self::Adt(a), Self::Adt(b)) => a.ctor == b.ctor && a.fields == b.fields,
            (Self::Record(a), Self::Record(b)) => a.ty == b.ty && a.fields == b.fields,
            (Self::Fn(a), Self::Fn(b)) | (Self::Ctor(a), Self::Ctor(b)) => a == b,
            (Self::Module(a), Self::Module(b)) => a == b,
            (Self::NativeFn(a), Self::NativeFn(b)) => a == b,
            (Self::Range(a), Self::Range(b)) => a == b,
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn equality_is_structural_and_kind_strict() {
        assert_eq!(
            Value::list(vec![Value::Int(1)]),
            Value::list(vec![Value::Int(1)])
        );
        assert_ne!(Value::Int(1), Value::Float(1.0));
        assert_eq!(Value::str("a"), Value::str("a"));
        assert_ne!(Value::tuple(vec![]), Value::Unit);
    }
}
