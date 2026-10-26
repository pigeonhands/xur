//! Runtime values.
//!
//! Xur is dynamically checked, so every value carries what it is. Lists and
//! native objects are shared and mutable, like Python's objects; everything
//! else is immutable. Cloning a value is always cheap.

use std::any::Any;
use std::cell::{Ref, RefCell, RefMut};
use std::fmt;
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
    /// A value of a registered Rust type, held as the Rust value itself.
    Native(Rc<NativeObject>),
    /// A value of an unregistered Rust struct, converted to named fields.
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

/// A value of a registered Rust type. Methods registered as taking
/// `&mut self` change it in place, and every Xur value referring to it sees
/// the change.
pub struct NativeObject {
    pub ty: NativeTypeId,
    value: RefCell<Box<dyn Any>>,
}

impl NativeObject {
    pub fn new(ty: NativeTypeId, value: Box<dyn Any>) -> Self {
        Self {
            ty,
            value: RefCell::new(value),
        }
    }

    /// The Rust value, if it is a `T`. Fails if it is borrowed mutably.
    pub fn borrow<T: 'static>(&self) -> Option<Ref<'_, T>> {
        let value = self.value.try_borrow().ok()?;
        Ref::filter_map(value, |value| value.downcast_ref()).ok()
    }

    /// The Rust value mutably, if it is a `T`. Fails if it is borrowed.
    pub fn borrow_mut<T: 'static>(&self) -> Option<RefMut<'_, T>> {
        let value = self.value.try_borrow_mut().ok()?;
        RefMut::filter_map(value, |value| value.downcast_mut()).ok()
    }

    /// The Rust value, type-erased.
    pub fn borrow_any(&self) -> Ref<'_, dyn Any> {
        Ref::map(self.value.borrow(), |value| value.as_ref())
    }
}

impl fmt::Debug for NativeObject {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("NativeObject").field(&self.ty).finish()
    }
}

/// A value of an unregistered Rust struct, converted to named fields.
#[derive(Debug, Clone, PartialEq)]
pub struct Record {
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
            Self::Native(_) => "native object",
            Self::Record(_) => "record",
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
            // Native objects are compared by identity here; the interpreter
            // compares their Rust values when the type supports it.
            (Self::Native(a), Self::Native(b)) => Rc::ptr_eq(a, b),
            (Self::Record(a), Self::Record(b)) => a == b,
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
