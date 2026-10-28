//! `std::ptr`: raw pointer construction and arithmetic.

use xur_runtime::{BuiltinType, Registry, RunResult, RuntimeError, Value};

pub(crate) fn register(registry: &Registry) {
    registry
        .module("std::ptr")
        .raw_function("null", |_cx, _args| Ok(Value::Ptr(0)))
        .named_raw_function("from_int", &["addr"], |_cx, args| {
            let [Value::Int(addr)] = args else {
                return Err(RuntimeError::new("`from_int` takes an integer"));
            };
            Ok(Value::Ptr(*addr as u64))
        });
    registry
        .builtin(BuiltinType::Ptr)
        .raw_function("to_int", |_cx, args| {
            let [Value::Ptr(addr)] = args else {
                return Err(RuntimeError::new("`to_int` takes a pointer"));
            };
            Ok(Value::Int(*addr as i64))
        })
        .raw_function("is_null", |_cx, args| {
            let [Value::Ptr(addr)] = args else {
                return Err(RuntimeError::new("`is_null` takes a pointer"));
            };
            Ok(Value::Bool(*addr == 0))
        })
        .raw_function("offset", |_cx, args| {
            let [Value::Ptr(addr), Value::Int(n)] = args else {
                return Err(RuntimeError::new("`offset` takes an integer"));
            };
            Ok(Value::Ptr(addr.wrapping_add(*n as u64)))
        })
        .raw_function("to_hex", |_cx, args| -> RunResult<Value> {
            let [Value::Ptr(addr)] = args else {
                return Err(RuntimeError::new("`to_hex` takes a pointer"));
            };
            Ok(Value::str(&format!("0x{addr:x}")))
        });
}
