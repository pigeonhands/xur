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

/// Locks two shared values for reading without cloning. When they are the same
/// value (aliased `Arc` — e.g. `xs.zip(xs)`), locks it once and hands the single
/// guard to both sides; otherwise locks each. This avoids the deadlock a plain
/// double `lock` of the same non-reentrant `Mutex` would hit.
pub fn with_pair<T: ?Sized, R>(
    a: &Arc<Mutex<T>>,
    b: &Arc<Mutex<T>>,
    f: impl FnOnce(&T, &T) -> R,
) -> R {
    if Arc::ptr_eq(a, b) {
        let g = lock(a);
        f(&g, &g)
    } else {
        let ga = lock(a);
        let gb = lock(b);
        f(&ga, &gb)
    }
}

/// An integer value that preserves its original width.
#[derive(Debug, Clone, Copy)]
pub enum IntValue {
    I8(i8),
    I16(i16),
    I32(i32),
    I64(i64),
    U8(u8),
    U16(u16),
    U32(u32),
    U64(u64),
}

impl IntValue {
    pub fn to_i128(self) -> i128 {
        match self {
            Self::I8(v) => v as i128,
            Self::I16(v) => v as i128,
            Self::I32(v) => v as i128,
            Self::I64(v) => v as i128,
            Self::U8(v) => v as i128,
            Self::U16(v) => v as i128,
            Self::U32(v) => v as i128,
            Self::U64(v) => v as i128,
        }
    }

    pub fn to_i64(self) -> Option<i64> {
        i64::try_from(self.to_i128()).ok()
    }

    pub fn to_u64(self) -> Option<u64> {
        u64::try_from(self.to_i128()).ok()
    }

    pub fn to_u32(self) -> Option<u32> {
        u32::try_from(self.to_i128()).ok()
    }

    pub fn to_usize(self) -> Option<usize> {
        usize::try_from(self.to_i128()).ok()
    }

    pub fn as_f64(self) -> f64 {
        self.to_i128() as f64
    }

    pub fn is_signed(self) -> bool {
        matches!(self, Self::I8(_) | Self::I16(_) | Self::I32(_) | Self::I64(_))
    }

    pub fn is_zero(self) -> bool {
        self.to_i128() == 0
    }

    pub fn width(self) -> u8 {
        match self {
            Self::I8(_) | Self::U8(_) => 1,
            Self::I16(_) | Self::U16(_) => 2,
            Self::I32(_) | Self::U32(_) => 4,
            Self::I64(_) | Self::U64(_) => 8,
        }
    }

    /// Try to narrow `result` into an IntValue of the given signedness and width.
    pub fn narrow(result: i128, signed: bool, width: u8) -> Option<Self> {
        if signed {
            match width {
                1 => i8::try_from(result).ok().map(Self::I8),
                2 => i16::try_from(result).ok().map(Self::I16),
                4 => i32::try_from(result).ok().map(Self::I32),
                _ => i64::try_from(result).ok().map(Self::I64),
            }
        } else {
            match width {
                1 => u8::try_from(result).ok().map(Self::U8),
                2 => u16::try_from(result).ok().map(Self::U16),
                4 => u32::try_from(result).ok().map(Self::U32),
                _ => u64::try_from(result).ok().map(Self::U64),
            }
        }
    }

    /// Narrow `result` to the common promoted type of two operands. Tries the
    /// natural type first, then widens, then flips signedness as a last resort.
    pub fn promote(a: Self, b: Self, result: i128) -> Option<Self> {
        let signed = a.is_signed() || b.is_signed();
        let width = a.width().max(b.width());
        Self::narrow(result, signed, width)
            .or_else(|| Self::narrow(result, signed, 8))
            .or_else(|| Self::narrow(result, !signed, 8))
    }

    pub fn checked_neg(self) -> Option<Self> {
        let result = self.to_i128().checked_neg()?;
        let width = self.width();
        Self::narrow(result, true, width)
            .or_else(|| Self::narrow(result, true, 8))
    }

    pub fn bit_not(self) -> Self {
        match self {
            Self::I8(v) => Self::I8(!v),
            Self::I16(v) => Self::I16(!v),
            Self::I32(v) => Self::I32(!v),
            Self::I64(v) => Self::I64(!v),
            Self::U8(v) => Self::U8(!v),
            Self::U16(v) => Self::U16(!v),
            Self::U32(v) => Self::U32(!v),
            Self::U64(v) => Self::U64(!v),
        }
    }
}

impl PartialEq for IntValue {
    fn eq(&self, other: &Self) -> bool {
        self.to_i128() == other.to_i128()
    }
}

impl Eq for IntValue {}

impl PartialOrd for IntValue {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for IntValue {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.to_i128().cmp(&other.to_i128())
    }
}

impl Hash for IntValue {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.to_i128().hash(state);
    }
}

impl fmt::Display for IntValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::I8(v) => write!(f, "{v}"),
            Self::I16(v) => write!(f, "{v}"),
            Self::I32(v) => write!(f, "{v}"),
            Self::I64(v) => write!(f, "{v}"),
            Self::U8(v) => write!(f, "{v}"),
            Self::U16(v) => write!(f, "{v}"),
            Self::U32(v) => write!(f, "{v}"),
            Self::U64(v) => write!(f, "{v}"),
        }
    }
}

impl From<i8> for IntValue {
    fn from(v: i8) -> Self { Self::I8(v) }
}
impl From<i16> for IntValue {
    fn from(v: i16) -> Self { Self::I16(v) }
}
impl From<i32> for IntValue {
    fn from(v: i32) -> Self { Self::I32(v) }
}
impl From<i64> for IntValue {
    fn from(v: i64) -> Self { Self::I64(v) }
}
impl From<u8> for IntValue {
    fn from(v: u8) -> Self { Self::U8(v) }
}
impl From<u16> for IntValue {
    fn from(v: u16) -> Self { Self::U16(v) }
}
impl From<u32> for IntValue {
    fn from(v: u32) -> Self { Self::U32(v) }
}
impl From<u64> for IntValue {
    fn from(v: u64) -> Self { Self::U64(v) }
}
impl From<usize> for IntValue {
    fn from(v: usize) -> Self { Self::U64(v as u64) }
}
impl From<bool> for IntValue {
    fn from(v: bool) -> Self { Self::U8(v as u8) }
}

/// A float value that preserves its original width.
#[derive(Debug, Clone, Copy)]
pub enum FloatValue {
    F32(f32),
    F64(f64),
}

impl FloatValue {
    pub fn as_f64(self) -> f64 {
        match self {
            Self::F32(v) => v as f64,
            Self::F64(v) => v,
        }
    }

    pub fn as_f32(self) -> f32 {
        match self {
            Self::F32(v) => v,
            Self::F64(v) => v as f32,
        }
    }
}

impl PartialEq for FloatValue {
    fn eq(&self, other: &Self) -> bool {
        self.as_f64() == other.as_f64()
    }
}

impl fmt::Display for FloatValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::F32(v) => write!(f, "{v}"),
            Self::F64(v) => write!(f, "{v}"),
        }
    }
}

impl From<f32> for FloatValue {
    fn from(v: f32) -> Self { Self::F32(v) }
}
impl From<f64> for FloatValue {
    fn from(v: f64) -> Self { Self::F64(v) }
}

#[derive(Debug, Clone)]
pub enum Value {
    /// `()`, the value of expressions that produce nothing.
    Unit,
    Bool(bool),
    Int(IntValue),
    Float(FloatValue),
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
    /// A value of a registered Rust enum type. Keeps the boxed Rust value
    /// (like `Native`) while exposing the active variant and fields so the
    /// VM can pattern-match on it.
    NativeEnum(Arc<NativeEnum>),
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
    /// Concrete type arguments for generic native types, e.g.
    /// `[BuiltinType::Bool]` for a `Cell<bool>`.
    type_args: Vec<BuiltinType>,
}

impl NativeObject {
    pub fn new(ty: NativeTypeId, value: Box<dyn Any + Send>) -> Self {
        Self {
            ty,
            value: Mutex::new(value),
            _keep_alive: None,
            type_args: Vec::new(),
        }
    }

    /// A native object with concrete type arguments for a generic type.
    pub fn with_type_args(
        ty: NativeTypeId,
        value: Box<dyn Any + Send>,
        type_args: Vec<BuiltinType>,
    ) -> Self {
        Self {
            ty,
            value: Mutex::new(value),
            _keep_alive: None,
            type_args,
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
            type_args: Vec::new(),
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

    /// The concrete type arguments of this object, e.g. `[Bool]` for
    /// `Cell<bool>`. Empty for non-generic types.
    pub fn type_args(&self) -> &[BuiltinType] {
        &self.type_args
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

/// A value of a registered Rust enum type. Keeps the boxed Rust value for
/// method dispatch and `from_value`, while exposing the active variant and
/// its fields so the VM can pattern-match on it.
pub struct NativeEnum {
    pub object: Arc<NativeObject>,
    pub variant: Symbol,
    pub fields: Vec<(Symbol, Value)>,
}

impl NativeEnum {
    pub fn field(&self, name: Symbol) -> Option<&Value> {
        self.fields
            .iter()
            .find(|(field, _)| *field == name)
            .map(|(_, value)| value)
    }
}

impl fmt::Debug for NativeEnum {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NativeEnum")
            .field("ty", &self.object.ty)
            .field("variant", &self.variant)
            .finish()
    }
}

impl PartialEq for NativeEnum {
    fn eq(&self, other: &Self) -> bool {
        self.object.ty == other.object.ty
            && self.variant == other.variant
            && self.fields == other.fields
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
    /// Convenience: `Value::int(42)` creates `Value::Int(IntValue::I64(42))`.
    pub fn int(v: i64) -> Self {
        Self::Int(IntValue::I64(v))
    }

    pub fn float(v: f64) -> Self {
        Self::Float(FloatValue::F64(v))
    }

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
            Self::NativeEnum(_) => "native enum",
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

impl fmt::Display for BuiltinType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Bool => "bool",
            Self::Int => "int",
            Self::Float => "float",
            Self::Str => "str",
            Self::Tuple => "tuple",
            Self::List => "list",
            Self::Map => "map",
            Self::Bytes => "bytes",
            Self::Ptr => "ptr",
            Self::Range => "range",
            Self::Record => "record",
            Self::Struct => "struct",
        })
    }
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
            (Self::NativeEnum(a), Self::NativeEnum(b)) => a == b,
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
            Value::list(vec![Value::int(1)]),
            Value::list(vec![Value::int(1)])
        );
        assert_ne!(Value::int(1), Value::float(1.0));
        assert_eq!(Value::str("a"), Value::str("a"));
        assert_eq!(Value::tuple(vec![]), Value::Unit);
    }

    #[test]
    fn int_value_cross_width_equality() {
        assert_eq!(IntValue::I64(42), IntValue::U8(42));
        assert_eq!(IntValue::U32(0), IntValue::I8(0));
        assert_ne!(IntValue::I64(-1), IntValue::U64(u64::MAX));
    }

    #[test]
    fn int_value_ordering() {
        assert!(IntValue::I64(-1) < IntValue::U8(0));
        assert!(IntValue::U64(u64::MAX) > IntValue::I64(i64::MAX));
    }

    #[test]
    fn int_value_conversions() {
        assert_eq!(IntValue::U64(u64::MAX).to_i64(), None);
        assert_eq!(IntValue::U64(42).to_i64(), Some(42));
        assert_eq!(IntValue::I64(-1).to_u64(), None);
        assert_eq!(IntValue::U8(255).to_u64(), Some(255));
    }

    #[test]
    fn float_value_cross_width_equality() {
        assert_eq!(FloatValue::F32(1.0), FloatValue::F64(1.0));
    }
}
