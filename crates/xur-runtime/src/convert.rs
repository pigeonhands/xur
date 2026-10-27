//! Converting between Rust values and Xur values, driven by facet shapes.
//!
//! | Rust                               | Xur                          |
//! | ---------------------------------- | ---------------------------- |
//! | `bool`                             | `bool`                       |
//! | integers                           | integers (range-checked)     |
//! | `f32`, `f64`                       | floats                       |
//! | `String`, `&str`, `char`           | strings                      |
//! | `()`                               | `()`                         |
//! | tuples                             | tuples                       |
//! | `Vec<u8>`                          | bytes                        |
//! | `Vec<T>`, arrays, slices           | lists                        |
//! | `Option<T>`, `Result<T, E>`        | the prelude's `Option`, `Result` |
//! | registered types                   | native objects               |
//! | other structs deriving `Facet`     | records with named fields    |

use facet::{Def, Facet, Partial, Peek, ScalarType, Shape, StructKind, StructType, Type, UserType};
use xur_span::Symbol;

use crate::value::{Adt, NativeObject, Record, lock};
use crate::{LangItems, Registry, RunResult, RuntimeError, Value};

/// What conversions need to know about the running program.
pub trait ConvertCx {
    fn lang(&self) -> &LangItems;
    /// The registry, for converting registered types.
    fn registry(&self) -> &Registry;
}

/// Converts a Rust value to a Xur value, cloning it if it is of a
/// registered type.
pub fn to_value<T: Facet<'static>>(cx: &dyn ConvertCx, value: &T) -> RunResult<Value> {
    peek_to_value(cx, Peek::new(value))
}

/// Converts a Rust value to a Xur value, moving it into a native object if
/// it is of a registered type.
pub fn into_value<T: Facet<'static> + Send + 'static>(
    cx: &dyn ConvertCx,
    value: T,
) -> RunResult<Value> {
    match cx.registry().type_of_shape(T::SHAPE) {
        Some(ty) => Ok(Value::Native(NativeObject::new(ty, Box::new(value)).into())),
        None => to_value(cx, &value),
    }
}

/// Converts a Xur value to a Rust value of type `T`, failing with a runtime
/// error if the value doesn't fit.
pub fn from_value<T: Facet<'static>>(cx: &dyn ConvertCx, value: &Value) -> RunResult<T> {
    let partial = Partial::alloc_owned::<T>().map_err(reflect_error)?;
    let partial = build(cx, partial, value)?;
    let heap_value = partial.build().map_err(reflect_error)?;
    heap_value.materialize::<T>().map_err(reflect_error)
}

pub(crate) fn peek_to_value(cx: &dyn ConvertCx, peek: Peek<'_, 'static>) -> RunResult<Value> {
    let shape = peek.shape();
    let registry = cx.registry();
    if let Some(ty) = registry.type_of_shape(shape) {
        let object = registry
            .object_from_peek(ty, peek)
            .ok_or_else(|| unsupported(shape))?;
        return Ok(Value::Native(object.into()));
    }
    if let Some(scalar) = peek.scalar_type() {
        return scalar_to_value(peek, scalar);
    }
    if shape == <Vec<u8>>::SHAPE {
        let bytes: &Vec<u8> = peek.get().map_err(reflect_error)?;
        return Ok(Value::bytes(bytes.clone()));
    }
    let lang = cx.lang();
    match shape.def {
        Def::Option(_) => {
            return Ok(match peek.into_option().map_err(reflect_error)?.value() {
                Some(inner) => Value::adt(lang.some, vec![peek_to_value(cx, inner)?]),
                None => Value::adt(lang.none, Vec::new()),
            });
        }
        Def::Result(_) => {
            let result = peek.into_result().map_err(reflect_error)?;
            return Ok(match (result.ok(), result.err()) {
                (Some(ok), _) => Value::adt(lang.ok, vec![peek_to_value(cx, ok)?]),
                (_, Some(err)) => Value::adt(lang.err, vec![peek_to_value(cx, err)?]),
                (None, None) => unreachable!("a result is either ok or err"),
            });
        }
        Def::List(_) | Def::Array(_) | Def::Slice(_) => {
            let items = peek.into_list_like().map_err(reflect_error)?;
            let items = items
                .iter()
                .map(|item| peek_to_value(cx, item))
                .collect::<RunResult<_>>()?;
            return Ok(Value::list(items));
        }
        _ => {}
    }
    match shape.ty {
        Type::User(UserType::Struct(StructType { kind, fields, .. })) => {
            let peek = peek.into_struct().map_err(reflect_error)?;
            let values = (0..fields.len())
                .map(|i| peek_to_value(cx, peek.field(i).map_err(reflect_error)?))
                .collect::<RunResult<Vec<_>>>()?;
            Ok(match kind {
                StructKind::Tuple => Value::tuple(values),
                StructKind::Unit | StructKind::Struct | StructKind::TupleStruct => {
                    let fields = fields
                        .iter()
                        .map(|field| Symbol::intern(field.name))
                        .zip(values)
                        .collect();
                    Value::Record(Record { fields }.into())
                }
            })
        }
        _ => Err(unsupported(shape)),
    }
}

fn scalar_to_value(peek: Peek<'_, 'static>, scalar: ScalarType) -> RunResult<Value> {
    fn get<T: Facet<'static> + Copy>(peek: Peek<'_, 'static>) -> RunResult<T> {
        peek.get::<T>().copied().map_err(reflect_error)
    }
    fn int<T: TryInto<i64> + std::fmt::Display + Copy>(value: T) -> RunResult<Value> {
        value
            .try_into()
            .map(Value::Int)
            .map_err(|_| RuntimeError::new(format!("{value} does not fit in a Xur integer")))
    }
    Ok(match scalar {
        ScalarType::Unit => Value::Unit,
        ScalarType::Bool => Value::Bool(get(peek)?),
        ScalarType::Char => Value::str(get::<char>(peek)?.encode_utf8(&mut [0; 4])),
        ScalarType::Str | ScalarType::String | ScalarType::CowStr => {
            Value::str(peek.as_str().ok_or_else(|| unsupported(peek.shape()))?)
        }
        ScalarType::F32 => Value::Float(get::<f32>(peek)?.into()),
        ScalarType::F64 => Value::Float(get(peek)?),
        ScalarType::U8 => int(get::<u8>(peek)?)?,
        ScalarType::U16 => int(get::<u16>(peek)?)?,
        ScalarType::U32 => int(get::<u32>(peek)?)?,
        ScalarType::U64 => int(get::<u64>(peek)?)?,
        ScalarType::U128 => int(get::<u128>(peek)?)?,
        ScalarType::USize => int(get::<usize>(peek)?)?,
        ScalarType::I8 => int(get::<i8>(peek)?)?,
        ScalarType::I16 => int(get::<i16>(peek)?)?,
        ScalarType::I32 => int(get::<i32>(peek)?)?,
        ScalarType::I64 => int(get::<i64>(peek)?)?,
        ScalarType::I128 => int(get::<i128>(peek)?)?,
        ScalarType::ISize => int(get::<isize>(peek)?)?,
        _ => return Err(unsupported(peek.shape())),
    })
}

/// A value of some Rust type under construction.
pub(crate) type Builder = Partial<'static, false>;

/// Fills in the value under construction in `partial` from `value`.
fn build(cx: &dyn ConvertCx, partial: Builder, value: &Value) -> RunResult<Builder> {
    let shape = partial.shape();
    let registry = cx.registry();
    if let Some(ty) = registry.type_of_shape(shape) {
        return match value {
            Value::Native(object) if object.ty == ty => registry.set_object(partial, object),
            _ => Err(mismatch(shape, value)),
        };
    }
    if let Some(scalar) = ScalarType::try_from_shape(shape) {
        return set_scalar(partial, scalar, value);
    }
    // A `Vec<u8>` takes bytes, or a list of byte values.
    if shape == <Vec<u8>>::SHAPE {
        let bytes = match value {
            Value::Bytes(bytes) => lock(bytes).clone(),
            Value::List(items) => lock(items)
                .iter()
                .map(|item| match item {
                    Value::Int(i) => u8::try_from(*i)
                        .map_err(|_| RuntimeError::new(format!("{i} does not fit in `u8`"))),
                    other => Err(mismatch(u8::SHAPE, other)),
                })
                .collect::<RunResult<_>>()?,
            _ => return Err(mismatch(shape, value)),
        };
        return partial.set(bytes).map_err(reflect_error);
    }
    let lang = cx.lang();
    match (&shape.def, value) {
        (Def::Option(_), Value::Adt(adt)) if adt.ctor == lang.some => {
            return build_inner(cx, partial.begin_some(), adt);
        }
        (Def::Option(_), Value::Adt(adt)) if adt.ctor == lang.none => {
            return partial.set_default().map_err(reflect_error);
        }
        (Def::Result(_), Value::Adt(adt)) if adt.ctor == lang.ok => {
            return build_inner(cx, partial.begin_ok(), adt);
        }
        (Def::Result(_), Value::Adt(adt)) if adt.ctor == lang.err => {
            return build_inner(cx, partial.begin_err(), adt);
        }
        (Def::Option(_) | Def::Result(_), _) => return Err(mismatch(shape, value)),
        (Def::List(_), Value::List(items)) => {
            let mut partial = partial.init_list().map_err(reflect_error)?;
            for item in lock(items).iter() {
                let element = partial.begin_list_item().map_err(reflect_error)?;
                partial = build(cx, element, item)?.end().map_err(reflect_error)?;
            }
            return Ok(partial);
        }
        (Def::List(_), _) => return Err(mismatch(shape, value)),
        _ => {}
    }
    match (shape.ty, value) {
        (Type::User(UserType::Struct(st)), Value::Tuple(items)) if st.kind == StructKind::Tuple => {
            if items.len() != st.fields.len() {
                return Err(mismatch(shape, value));
            }
            build_fields(cx, partial, items.iter())
        }
        (Type::User(UserType::Struct(st)), Value::Record(record))
            if st.kind != StructKind::Tuple =>
        {
            let values = st
                .fields
                .iter()
                .map(|field| {
                    record.field(Symbol::intern(field.name)).ok_or_else(|| {
                        RuntimeError::new(format!("missing field `{}` for `{shape}`", field.name))
                    })
                })
                .collect::<RunResult<Vec<_>>>()?;
            build_fields(cx, partial, values.into_iter())
        }
        _ => Err(mismatch(shape, value)),
    }
}

/// Builds the single field of a `Some`, `Ok` or `Err`, entered with `begin`.
fn build_inner(
    cx: &dyn ConvertCx,
    begin: Result<Builder, facet::ReflectError>,
    adt: &Adt,
) -> RunResult<Builder> {
    let inner = begin.map_err(reflect_error)?;
    build(cx, inner, &adt.fields[0])?
        .end()
        .map_err(reflect_error)
}

fn build_fields<'v>(
    cx: &dyn ConvertCx,
    mut partial: Builder,
    values: impl Iterator<Item = &'v Value>,
) -> RunResult<Builder> {
    for (i, value) in values.enumerate() {
        let field = partial.begin_nth_field(i).map_err(reflect_error)?;
        partial = build(cx, field, value)?.end().map_err(reflect_error)?;
    }
    Ok(partial)
}

fn set_scalar(partial: Builder, scalar: ScalarType, value: &Value) -> RunResult<Builder> {
    let shape = partial.shape();
    macro_rules! int {
        ($ty:ty) => {{
            let Value::Int(int) = *value else {
                return Err(mismatch(shape, value));
            };
            let int = <$ty>::try_from(int).map_err(|_| {
                RuntimeError::new(format!("{int} does not fit in `{}`", stringify!($ty)))
            })?;
            partial.set(int)
        }};
    }
    let result = match (scalar, value) {
        (ScalarType::Unit, Value::Unit) => partial.set(()),
        (ScalarType::Bool, &Value::Bool(b)) => partial.set(b),
        (ScalarType::String, Value::Str(s)) => partial.set(s.to_string()),
        (ScalarType::Char, Value::Str(s)) if s.chars().count() == 1 => {
            partial.set(s.chars().next().expect("one char"))
        }
        (ScalarType::F64, &Value::Float(f)) => partial.set(f),
        (ScalarType::F32, &Value::Float(f)) => partial.set(f as f32),
        (ScalarType::U8, _) => int!(u8),
        (ScalarType::U16, _) => int!(u16),
        (ScalarType::U32, _) => int!(u32),
        (ScalarType::U64, _) => int!(u64),
        (ScalarType::U128, _) => int!(u128),
        (ScalarType::USize, _) => int!(usize),
        (ScalarType::I8, _) => int!(i8),
        (ScalarType::I16, _) => int!(i16),
        (ScalarType::I32, _) => int!(i32),
        (ScalarType::I64, _) => int!(i64),
        (ScalarType::I128, _) => int!(i128),
        (ScalarType::ISize, _) => int!(isize),
        _ => return Err(mismatch(shape, value)),
    };
    result.map_err(reflect_error)
}

fn mismatch(shape: &Shape, value: &Value) -> RuntimeError {
    RuntimeError::new(format!("expected `{shape}`, found {}", value.kind()))
}

fn unsupported(shape: &Shape) -> RuntimeError {
    RuntimeError::new(format!("`{shape}` cannot be converted to a Xur value"))
}

pub(crate) fn reflect_error(error: impl std::fmt::Display) -> RuntimeError {
    RuntimeError::new(format!("conversion failed: {error}"))
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use xur_resolve::Program;
    use xur_span::MemoryFileLoader;

    use super::*;

    #[derive(Facet, Debug, Clone, PartialEq)]
    struct Point {
        x: f64,
        label: String,
    }

    #[derive(Facet, Debug, Clone, PartialEq)]
    struct Label {
        text: String,
    }

    /// Registers `Point`; `Label` stays unregistered.
    struct Cx(LangItems, Registry);

    impl ConvertCx for Cx {
        fn lang(&self) -> &LangItems {
            &self.0
        }

        fn registry(&self) -> &Registry {
            &self.1
        }
    }

    fn cx() -> Cx {
        let loader = MemoryFileLoader::new([("main.xur", "")]);
        let (program, _) = Program::load(
            Path::new("main.xur"),
            Box::new(loader),
            Box::new(Vec::new()),
        )
        .unwrap();
        let registry = Registry::new();
        registry.module("geo").ty::<Point>("Point");
        Cx(program.lang_items(), registry)
    }

    fn round_trip<T: Facet<'static> + PartialEq + std::fmt::Debug>(value: T) -> Value {
        let cx = cx();
        let converted = to_value(&cx, &value).unwrap();
        assert_eq!(from_value::<T>(&cx, &converted).unwrap(), value);
        converted
    }

    #[test]
    fn scalars() {
        assert_eq!(round_trip(true), Value::Bool(true));
        assert_eq!(round_trip(42u8), Value::Int(42));
        assert_eq!(round_trip(-7i64), Value::Int(-7));
        assert_eq!(round_trip(1.5f64), Value::Float(1.5));
        assert_eq!(round_trip(String::from("hi")), Value::str("hi"));
        assert_eq!(round_trip('x'), Value::str("x"));
        assert_eq!(round_trip(()), Value::Unit);
    }

    #[test]
    fn compound_values() {
        assert_eq!(
            round_trip(vec![1u32, 2]),
            Value::list(vec![Value::Int(1), Value::Int(2)])
        );
        assert_eq!(
            round_trip((1i32, String::from("a"))),
            Value::tuple(vec![Value::Int(1), Value::str("a")])
        );
        let lang = cx().0;
        assert_eq!(
            round_trip(Some(3u8)),
            Value::adt(lang.some, vec![Value::Int(3)])
        );
        assert_eq!(round_trip(None::<u8>), Value::adt(lang.none, vec![]));
        assert_eq!(
            round_trip(Err::<u8, String>("bad".into())),
            Value::adt(lang.err, vec![Value::str("bad")])
        );
    }

    #[test]
    fn byte_vectors_are_bytes() {
        assert_eq!(round_trip(vec![1u8, 255]), Value::bytes(vec![1, 255]));
    }

    #[test]
    fn registered_types_become_native_objects() {
        let point = Point {
            x: 1.0,
            label: "p".into(),
        };
        let cx = cx();
        let value = into_value(&cx, point.clone()).unwrap();
        let Value::Native(object) = &value else {
            panic!("expected a native object, found {value:?}");
        };
        assert_eq!(object.with(Point::clone), Some(point.clone()));
        assert_eq!(from_value::<Point>(&cx, &value).unwrap(), point);
        // Nested in other data, they are cloned in and out.
        let list = to_value(&cx, &vec![point.clone()]).unwrap();
        assert_eq!(from_value::<Vec<Point>>(&cx, &list).unwrap(), [point]);
    }

    #[test]
    fn other_structs_become_records() {
        let value = round_trip(Label { text: "l".into() });
        let Value::Record(record) = value else {
            panic!("expected a record, found {value:?}");
        };
        assert_eq!(record.field(Symbol::intern("text")), Some(&Value::str("l")));
    }

    #[test]
    fn out_of_range_integers_are_errors() {
        let error = from_value::<u8>(&cx(), &Value::Int(300)).unwrap_err();
        assert_eq!(error.message, "300 does not fit in `u8`");
    }

    #[test]
    fn mismatched_values_are_errors() {
        let error = from_value::<u32>(&cx(), &Value::str("1")).unwrap_err();
        assert_eq!(error.message, "expected `u32`, found string");
        let error = from_value::<Vec<u32>>(&cx(), &Value::Int(1)).unwrap_err();
        assert_eq!(error.message, "expected `Vec<u32>`, found integer");
    }
}
