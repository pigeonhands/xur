//! Runtime values.
//!
//! Xur is dynamically checked, so every value carries what it is. Lists and
//! native objects are shared and mutable, like Python's objects; everything
//! else is immutable. Cloning a value is always cheap.
//!
//! Values are `Send + Sync` so threads can share them: shared mutable values
//! are behind a [`Mutex`].

use std::any::Any;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use dashmap::DashMap;

use xur_resolve::{DefId, NativeFnId, NativeTypeId, RtType};
use xur_span::Symbol;

/// Locks `mutex`. A panic while it was held doesn't stop the program from
/// using the value, as Xur values have no invariants a panic could break.
pub fn lock<T: ?Sized>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

#[derive(Debug, Clone)]
pub enum Value {
    /// `()`, the value of expressions that produce nothing.
    Unit,
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(Arc<str>),
    Tuple(Arc<[Value]>),
    List(Arc<Mutex<Vec<Value>>>),
    /// `#{k: v}`, shared and mutable like a list.
    Map(Arc<DashMap<MapKey, Value>>),
    /// A byte buffer, shared and mutable like a list (Python's `bytearray`).
    Bytes(Arc<Mutex<Vec<u8>>>),
    /// A raw pointer (address), for memory-level operations.
    Ptr(u64),
    /// An instance of a struct or enum variant defined in Xur.
    Adt(Arc<Adt>),
    /// A value of a registered Rust type, held as the Rust value itself.
    Native(Arc<NativeObject>),
    /// A value of an unregistered Rust struct, converted to named fields.
    Record(Arc<Record>),
    /// A function defined in Xur, including those in `impl` blocks.
    Fn(DefId),
    /// The constructor of a tuple struct or tuple variant used as a
    /// function, as in `Some(1)` or `list.map(Some)`.
    Ctor(DefId),
    /// A function implemented in Rust.
    NativeFn(NativeFnId),
    /// A closure: code owned by the engine running it, and what it captured.
    Closure(Arc<Closure>),
    /// An iterator, from iterating a list or range or from a native function.
    Iterator(Arc<ValueIter>),
    /// A module: modules are values.
    Module(DefId),
    /// `a..b`, `a..=b`, `a..`, `..b` over integers.
    Range(Range),
    /// A type used as a value: `u32`, `fn(u32) -> str`, etc.
    Type(Box<RtType>),
}

/// An instance of a struct, or of an enum variant, defined in Xur.
#[derive(Debug, Clone)]
pub struct Adt {
    /// The struct or variant.
    pub ctor: DefId,
    pub fields: Vec<Value>,
}

/// A closure value.
#[derive(Debug)]
pub struct Closure {
    /// The closure's code, in whatever form the engine running it uses.
    pub code: Arc<dyn Any + Send + Sync>,
    /// The values of the variables it captured when it was created.
    /// Wrapped in a Mutex so repeated calls can write back mutations.
    pub captures: Mutex<Vec<Value>>,
    /// The type arguments in effect where it was created.
    pub type_args: Vec<crate::TypeRef>,
}

/// A shared iterator over values.
pub struct ValueIter(Mutex<Box<dyn Iterator<Item = Value> + Send>>);

impl ValueIter {
    pub fn new(iter: impl Iterator<Item = Value> + Send + 'static) -> Self {
        Self(Mutex::new(Box::new(iter)))
    }

    #[allow(clippy::should_implement_trait)] // it is shared, so `&self`
    pub fn next(&self) -> Option<Value> {
        lock(&self.0).next()
    }
}

impl fmt::Debug for ValueIter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ValueIter")
    }
}

/// A value of a registered Rust type. Methods registered as taking
/// `&mut self` change it in place, and every Xur value referring to it sees
/// the change.
pub struct NativeObject {
    pub ty: NativeTypeId,
    value: Mutex<Box<dyn Any + Send>>,
    /// Memory the Rust value borrows, kept alive for as long as the object:
    /// the file bytes behind a parsed `PEFile<'static>`, for instance. The
    /// value's `'static` references stay valid because this never moves or
    /// drops while the value does.
    _keep_alive: Option<Arc<dyn Any + Send + Sync>>,
}

impl NativeObject {
    pub fn new(ty: NativeTypeId, value: Box<dyn Any + Send>) -> Self {
        Self {
            ty,
            value: Mutex::new(value),
            _keep_alive: None,
        }
    }

    /// A native object whose value borrows `keep_alive`, which is held as
    /// long as the object so those borrows stay valid.
    pub fn borrowing(
        ty: NativeTypeId,
        value: Box<dyn Any + Send>,
        keep_alive: Arc<dyn Any + Send + Sync>,
    ) -> Self {
        Self {
            ty,
            value: Mutex::new(value),
            _keep_alive: Some(keep_alive),
        }
    }

    /// Calls `f` with the Rust value, if it is a `T`.
    pub fn with<T: 'static, R>(&self, f: impl FnOnce(&T) -> R) -> Option<R> {
        lock(&self.value).downcast_ref().map(f)
    }

    /// Calls `f` with the Rust value mutably, if it is a `T`.
    pub fn with_mut<T: 'static, R>(&self, f: impl FnOnce(&mut T) -> R) -> Option<R> {
        lock(&self.value).downcast_mut().map(f)
    }

    /// The memory this object keeps alive for its value's borrows, to share
    /// with a part of it parsed from the same bytes.
    pub fn keep_alive(&self) -> Option<Arc<dyn Any + Send + Sync>> {
        self._keep_alive.clone()
    }

    /// Calls `f` with the Rust value, type-erased.
    pub fn with_any<R>(&self, f: impl FnOnce(&dyn Any) -> R) -> R {
        f(lock(&self.value).as_ref())
    }
}

impl fmt::Debug for NativeObject {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("NativeObject").field(&self.ty).finish()
    }
}

/// Named fields: a value of a Rust struct or enum converted to Xur, or a
/// record read with a layout.
#[derive(Debug, Clone, PartialEq)]
pub struct Record {
    /// The type registered for the Rust type's shape, if any, which names
    /// it and is what written types check against.
    pub ty: Option<NativeTypeId>,
    /// For a value of a Rust enum, its variant, e.g. `Amd64`.
    pub variant: Option<Symbol>,
    pub fields: Vec<(Symbol, Value)>,
}

impl Record {
    /// A record of no particular type.
    pub fn new(fields: Vec<(Symbol, Value)>) -> Self {
        Self {
            ty: None,
            variant: None,
            fields,
        }
    }

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
        Self::List(Arc::new(Mutex::new(items)))
    }

    pub fn map(entries: Vec<(Value, Value)>) -> Self {
        let map = DashMap::new();
        for (k, v) in entries {
            map.insert(MapKey(k), v);
        }
        Self::Map(Arc::new(map))
    }

    pub fn bytes(bytes: Vec<u8>) -> Self {
        Self::Bytes(Arc::new(Mutex::new(bytes)))
    }

    pub fn tuple(items: Vec<Value>) -> Self {
        if items.is_empty() {
            Self::Unit
        } else {
            Self::Tuple(items.into())
        }
    }

    pub fn adt(ctor: DefId, fields: Vec<Value>) -> Self {
        Self::Adt(Arc::new(Adt { ctor, fields }))
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
            Self::Map(_) => "map",
            Self::Bytes(_) => "bytes",
            Self::Ptr(_) => "pointer",
            Self::Adt(_) => "struct or enum",
            Self::Native(_) => "native object",
            Self::Record(_) => "record",
            Self::Fn(_) | Self::NativeFn(_) => "function",
            Self::Closure(_) => "closure",
            Self::Iterator(_) => "iterator",
            Self::Ctor(_) => "constructor",
            Self::Module(_) => "module",
            Self::Range(_) => "range",
            Self::Type(_) => "type",
        }
    }
}

/// The kinds of built-in value that have methods, like `list.push(x)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BuiltinType {
    Bool,
    Int,
    Float,
    Str,
    Tuple,
    List,
    Map,
    Bytes,
    Ptr,
    Range,
    Record,
    /// Every struct and enum variant defined in Xur, for methods they all
    /// have, like `to_bytes`. Their own `impl` functions come first.
    Struct,
}

impl Value {
    /// The built-in type whose methods `value.method()` looks up.
    pub fn builtin_type(&self) -> Option<BuiltinType> {
        Some(match self {
            Self::Bool(_) => BuiltinType::Bool,
            Self::Int(_) => BuiltinType::Int,
            Self::Float(_) => BuiltinType::Float,
            Self::Str(_) => BuiltinType::Str,
            Self::Tuple(_) => BuiltinType::Tuple,
            Self::List(_) => BuiltinType::List,
            Self::Map(_) => BuiltinType::Map,
            Self::Bytes(_) => BuiltinType::Bytes,
            Self::Ptr(_) => BuiltinType::Ptr,
            Self::Range(_) => BuiltinType::Range,
            Self::Record(_) => BuiltinType::Record,
            Self::Adt(_) => BuiltinType::Struct,
            _ => return None,
        })
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
            (Self::List(a), Self::List(b)) => Arc::ptr_eq(a, b) || *lock(a) == *lock(b),
            (Self::Map(a), Self::Map(b)) => {
                Arc::ptr_eq(a, b)
                    || (a.len() == b.len()
                        && a.iter().all(|entry| {
                            b.get(entry.key())
                                .is_some_and(|v| *v.value() == *entry.value())
                        }))
            }
            (Self::Bytes(a), Self::Bytes(b)) => Arc::ptr_eq(a, b) || *lock(a) == *lock(b),
            (Self::Ptr(a), Self::Ptr(b)) => a == b,
            (Self::Adt(a), Self::Adt(b)) => a.ctor == b.ctor && a.fields == b.fields,
            // Native objects are compared by identity here; the interpreter
            // compares their Rust values when the type supports it.
            (Self::Native(a), Self::Native(b)) => Arc::ptr_eq(a, b),
            (Self::Record(a), Self::Record(b)) => a == b,
            (Self::Fn(a), Self::Fn(b)) | (Self::Ctor(a), Self::Ctor(b)) => a == b,
            (Self::Module(a), Self::Module(b)) => a == b,
            (Self::NativeFn(a), Self::NativeFn(b)) => a == b,
            (Self::Range(a), Self::Range(b)) => a == b,
            (Self::Type(a), Self::Type(b)) => a == b,
            (Self::Closure(a), Self::Closure(b)) => Arc::ptr_eq(a, b),
            (Self::Iterator(a), Self::Iterator(b)) => Arc::ptr_eq(a, b),
            _ => false,
        }
    }
}

/// A value used as a map key. Only hashable types (str, int, bool) are
/// accepted; unhashable ones (list, map, etc.) will panic if used as keys.
#[derive(Debug, Clone)]
pub struct MapKey(pub Value);

impl PartialEq for MapKey {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}

impl Eq for MapKey {}

impl Hash for MapKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        std::mem::discriminant(&self.0).hash(state);
        match &self.0 {
            Value::Unit => {}
            Value::Bool(b) => b.hash(state),
            Value::Int(i) => i.hash(state),
            Value::Str(s) => s.hash(state),
            Value::Adt(adt) if adt.fields.is_empty() => adt.ctor.hash(state),
            other => panic!("{} is not hashable", other.kind()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_can_be_shared_between_threads() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<Value>();
    }

    #[test]
    fn equality_is_structural_and_kind_strict() {
        assert_eq!(
            Value::list(vec![Value::Int(1)]),
            Value::list(vec![Value::Int(1)])
        );
        assert_ne!(Value::Int(1), Value::Float(1.0));
        assert_eq!(Value::str("a"), Value::str("a"));
        assert_eq!(Value::tuple(vec![]), Value::Unit);
    }
}
