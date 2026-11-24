//! Methods of `bytes`, and `std::bytes` for making them.
//!
//! Integers are read and written at byte offsets in either byte order:
//! `data.read_u32_le(0x3c)`, `data.write_u16_be(4, 0xbeef)`. Methods that
//! change the buffer change it in place.

use std::sync::{Arc, Mutex};

use xur_runtime::{BuiltinType, Registry, RunResult, RuntimeError, Value, lock};
use xur_runtime::registry::XurType;

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
        .method("len", |bytes: &[u8]| -> i64 { bytes.len() as i64 })
        .method("is_empty", |bytes: &[u8]| -> bool { bytes.is_empty() })
        .method_mut("push", |bytes: &mut Vec<u8>, byte: i64| {
            bytes.push(byte as u8);
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
        .method("find", |bytes: &[u8], needle: Vec<u8>| -> Option<i64> {
            find(bytes, &needle).map(|at| at as i64)
        })
        .method("starts_with", |bytes: &[u8], prefix: Vec<u8>| -> bool {
            bytes.starts_with(&prefix)
        })
        .method("to_hex", |bytes: &[u8]| -> String {
            bytes.iter().map(|byte| format!("{byte:02x}")).collect()
        })
        .method("decode", |bytes: &[u8]| -> Result<String, String> {
            String::from_utf8(bytes.to_vec()).map_err(|e| e.to_string())
        })
        .method("to_list", |bytes: &[u8]| -> Vec<i64> {
            bytes.iter().map(|&b| b as i64).collect()
        })
        .method("get", |bytes: &[u8], index: i64| -> Option<i64> {
            usize::try_from(index)
                .ok()
                .and_then(|i| bytes.get(i).copied())
                .map(|b| b as i64)
        })
        .method("slice", |bytes: &[u8], start: i64, end: i64| -> Result<Vec<u8>, String> {
            let s = usize::try_from(start).map_err(|_| "start out of range".to_string())?;
            let e = usize::try_from(end).map_err(|_| "end out of range".to_string())?;
            if s > e || e > bytes.len() {
                return Err(format!("slice [{s}..{e}] out of range for {} bytes", bytes.len()));
            }
            Ok(bytes[s..e].to_vec())
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
            i.to_usize().ok_or_else(|| RuntimeError::new(format!("{i} is not a valid offset")))
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
            .map(Value::int)
            .map_err(|_| RuntimeError::new(format!("{i} does not fit in a Xur integer"))),
        Number::Float(f) => Ok(Value::float(f)),
    }
}

/// A Xur value as a number of type `T`, named `name` in errors.
fn number_from<T: TryFrom<Number>>(value: &Value, name: &str) -> RunResult<T> {
    let (number, shown) = match value {
        Value::Int(i) => (Number::Int(i.to_i128()), i.to_string()),
        Value::Float(f) => (Number::Float(f.as_f64()), f.to_string()),
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
