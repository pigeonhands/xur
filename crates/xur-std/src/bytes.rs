//! Methods of `bytes`, and `std::bytes` for making them.
//!
//! Integers are read and written at byte offsets in either byte order:
//! `data.read_u32_le(0x3c)`, `data.write_u16_be(4, 0xbeef)`. Methods that
//! change the buffer change it in place.

use std::sync::{Arc, Mutex};

use xur_runtime::{BuiltinType, Registry, RunResult, RuntimeError, Value, lock};
use xur_runtime::registry::XurType;

use crate::option;

type Bytes = Arc<Mutex<Vec<u8>>>;

pub(crate) fn register(registry: &Registry) {
    registry
        .module("std::bytes")
        .named_function("zeroed", &["len"], |len: usize| vec![0u8; len])
        .named_function("from_list", &["bytes"], |bytes: Vec<u8>| bytes)
        .named_function("from_hex", &["hex"], from_hex);
    registry
        .builtin(BuiltinType::Str)
        .function("to_bytes", |s: String| s.into_bytes())
        .function("from_hex", from_hex);

    let mut methods = registry.builtin(BuiltinType::Bytes);
    methods
        .raw_function_returns::<i64>("len", |_, args| {
            let (bytes, []) = receiver(args)? else {
                return arity(0);
            };
            Ok(Value::Int(lock(bytes).len() as i64))
        })
        .raw_function_returns::<bool>("is_empty", |_, args| {
            let (bytes, []) = receiver(args)? else {
                return arity(0);
            };
            Ok(Value::Bool(lock(bytes).is_empty()))
        })
        .raw_function_returns_xur("push", XurType::UNIT, |_, args| {
            let (bytes, [byte]) = receiver(args)? else {
                return arity(1);
            };
            let byte = to_byte(byte)?;
            lock(bytes).push(byte);
            Ok(Value::Unit)
        })
        .raw_function_returns_xur("extend", XurType::UNIT, |_, args| {
            let (bytes, [Value::Bytes(other)]) = receiver(args)? else {
                return Err(RuntimeError::new("`extend` takes bytes"));
            };
            // Copy first: `data.extend(data)` locks the same buffer twice.
            let other = lock(other).clone();
            lock(bytes).extend(other);
            Ok(Value::Unit)
        })
        .raw_function("find", |cx, args| {
            let (bytes, [Value::Bytes(needle)]) = receiver(args)? else {
                return Err(RuntimeError::new("`find` takes the bytes to look for"));
            };
            let needle = lock(needle).clone();
            let found = find(&lock(bytes), &needle).map(|at| Value::Int(at as i64));
            Ok(option(cx, found))
        })
        .raw_function_returns::<bool>("starts_with", |_, args| {
            let (bytes, [Value::Bytes(prefix)]) = receiver(args)? else {
                return Err(RuntimeError::new("`starts_with` takes bytes"));
            };
            let prefix = lock(prefix).clone();
            Ok(Value::Bool(lock(bytes).starts_with(&prefix)))
        })
        .raw_function_returns::<String>("to_hex", |_, args| {
            let (bytes, []) = receiver(args)? else {
                return arity(0);
            };
            let hex: String = lock(bytes)
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect();
            Ok(Value::str(&hex))
        })
        .raw_function("decode", |cx, args| {
            let (bytes, []) = receiver(args)? else {
                return arity(0);
            };
            let lang = cx.lang();
            Ok(match String::from_utf8(lock(bytes).clone()) {
                Ok(text) => Value::adt(lang.ok, vec![Value::str(&text)]),
                Err(error) => Value::adt(lang.err, vec![Value::str(&error.to_string())]),
            })
        })
        .raw_function_returns_xur("to_list", XurType::LIST, |_, args| {
            let (bytes, []) = receiver(args)? else {
                return arity(0);
            };
            let items = lock(bytes)
                .iter()
                .map(|&byte| Value::Int(byte.into()))
                .collect();
            Ok(Value::list(items))
        });

    macro_rules! numbers {
        ($($ty:ty: $read:literal, $write:literal, $from:ident, $to:ident;)*) => {$(
            methods
                .raw_function($read, |_, args| {
                    let (bytes, [offset]) = receiver(args)? else { return arity(1) };
                    let raw = read::<{ size_of::<$ty>() }>(&lock(bytes), offset)?;
                    number_value(<$ty>::$from(raw))
                })
                .raw_function($write, |_, args| {
                    let (bytes, [offset, value]) = receiver(args)? else { return arity(2) };
                    let value: $ty = number_from(value, stringify!($ty))?;
                    write(&mut lock(bytes), offset, &value.$to())
                });
        )*};
    }
    numbers! {
        u8: "read_u8", "write_u8", from_le_bytes, to_le_bytes;
        i8: "read_i8", "write_i8", from_le_bytes, to_le_bytes;
        u16: "read_u16_le", "write_u16_le", from_le_bytes, to_le_bytes;
        u16: "read_u16_be", "write_u16_be", from_be_bytes, to_be_bytes;
        i16: "read_i16_le", "write_i16_le", from_le_bytes, to_le_bytes;
        i16: "read_i16_be", "write_i16_be", from_be_bytes, to_be_bytes;
        u32: "read_u32_le", "write_u32_le", from_le_bytes, to_le_bytes;
        u32: "read_u32_be", "write_u32_be", from_be_bytes, to_be_bytes;
        i32: "read_i32_le", "write_i32_le", from_le_bytes, to_le_bytes;
        i32: "read_i32_be", "write_i32_be", from_be_bytes, to_be_bytes;
        u64: "read_u64_le", "write_u64_le", from_le_bytes, to_le_bytes;
        u64: "read_u64_be", "write_u64_be", from_be_bytes, to_be_bytes;
        i64: "read_i64_le", "write_i64_le", from_le_bytes, to_le_bytes;
        i64: "read_i64_be", "write_i64_be", from_be_bytes, to_be_bytes;
        f32: "read_f32_le", "write_f32_le", from_le_bytes, to_le_bytes;
        f32: "read_f32_be", "write_f32_be", from_be_bytes, to_be_bytes;
        f64: "read_f64_le", "write_f64_le", from_le_bytes, to_le_bytes;
        f64: "read_f64_be", "write_f64_be", from_be_bytes, to_be_bytes;
    }
}

/// Parses pairs of hex digits, ignoring whitespace: `"4d 5a"`.
pub(crate) fn from_hex(hex: String) -> Result<Vec<u8>, String> {
    let digits: Vec<char> = hex.chars().filter(|c| !c.is_whitespace()).collect();
    if !digits.len().is_multiple_of(2) {
        return Err("hex strings need two digits per byte".to_owned());
    }
    digits
        .chunks(2)
        .map(|pair| {
            let pair: String = pair.iter().collect();
            u8::from_str_radix(&pair, 16).map_err(|_| format!("`{pair}` is not a hex byte"))
        })
        .collect()
}

/// Splits a method call's arguments into the bytes and the rest.
fn receiver(args: &[Value]) -> RunResult<(&Bytes, &[Value])> {
    match args {
        [Value::Bytes(bytes), rest @ ..] => Ok((bytes, rest)),
        _ => Err(RuntimeError::new("expected bytes")),
    }
}

fn arity(expected: usize) -> RunResult<Value> {
    Err(RuntimeError::new(format!(
        "expected {expected} argument(s)"
    )))
}

fn to_byte(value: &Value) -> RunResult<u8> {
    number_from(value, "u8")
}

/// The `N` bytes at `offset`.
fn read<const N: usize>(bytes: &[u8], offset: &Value) -> RunResult<[u8; N]> {
    let start = offset_of(offset)?;
    bytes
        .get(start..start + N)
        .and_then(|slice| slice.try_into().ok())
        .ok_or_else(|| out_of_range(start, N, bytes.len()))
}

fn write(bytes: &mut [u8], offset: &Value, raw: &[u8]) -> RunResult<Value> {
    let start = offset_of(offset)?;
    let len = bytes.len();
    let target = bytes
        .get_mut(start..start + raw.len())
        .ok_or_else(|| out_of_range(start, raw.len(), len))?;
    target.copy_from_slice(raw);
    Ok(Value::Unit)
}

fn offset_of(offset: &Value) -> RunResult<usize> {
    match offset {
        Value::Int(i) => {
            usize::try_from(*i).map_err(|_| RuntimeError::new(format!("{i} is not a valid offset")))
        }
        other => Err(RuntimeError::new(format!(
            "expected an integer offset, found {}",
            other.kind()
        ))),
    }
}

fn out_of_range(start: usize, len: usize, total: usize) -> RuntimeError {
    RuntimeError::new(format!(
        "{len} byte(s) at offset {start} are out of range for {total} byte(s)"
    ))
}

/// A number read from bytes, as a Xur value.
fn number_value<T: Into<Number>>(value: T) -> RunResult<Value> {
    match value.into() {
        Number::Int(i) => i64::try_from(i)
            .map(Value::Int)
            .map_err(|_| RuntimeError::new(format!("{i} does not fit in a Xur integer"))),
        Number::Float(f) => Ok(Value::Float(f)),
    }
}

/// A Xur value as a number of type `T`, named `name` in errors.
fn number_from<T: TryFrom<Number>>(value: &Value, name: &str) -> RunResult<T> {
    let (number, shown) = match value {
        Value::Int(i) => (Number::Int((*i).into()), i.to_string()),
        Value::Float(f) => (Number::Float(*f), f.to_string()),
        other => {
            return Err(RuntimeError::new(format!(
                "expected a number, found {}",
                other.kind()
            )));
        }
    };
    T::try_from(number).map_err(|_| RuntimeError::new(format!("{shown} does not fit in `{name}`")))
}

/// Any number that can be read or written.
enum Number {
    Int(i128),
    Float(f64),
}

macro_rules! number_conversions {
    (ints: $($int:ty),*; floats: $($float:ty),*) => {
        $(
            impl From<$int> for Number {
                fn from(value: $int) -> Self {
                    Self::Int(value.into())
                }
            }
            impl TryFrom<Number> for $int {
                type Error = ();
                fn try_from(number: Number) -> Result<Self, ()> {
                    match number {
                        Number::Int(i) => i.try_into().map_err(|_| ()),
                        Number::Float(_) => Err(()),
                    }
                }
            }
        )*
        $(
            impl From<$float> for Number {
                fn from(value: $float) -> Self {
                    Self::Float(value.into())
                }
            }
            impl TryFrom<Number> for $float {
                type Error = ();
                fn try_from(number: Number) -> Result<Self, ()> {
                    match number {
                        Number::Float(f) => Ok(f as $float),
                        Number::Int(_) => Err(()),
                    }
                }
            }
        )*
    };
}

number_conversions!(ints: u8, i8, u16, i16, u32, i32, u64, i64; floats: f32, f64);

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() {
        return Some(0);
    }
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}
