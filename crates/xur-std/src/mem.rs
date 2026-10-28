//! `std::mem`: sizes, alignment and byte-level conversion of structs.

use std::sync::Arc;

use xur_runtime::{BuiltinType, Context, Registry, RunResult, RuntimeError, Value, lock};

use crate::reflect::layout::{layout_of_def, type_arg_layout};

pub(crate) fn register(registry: &Registry) {
    registry
        .module("std::mem")
        .named_raw_function("size_of", &[], |cx, args| {
            let layout = type_arg_layout(cx, args, "size_of")?;
            Ok(Value::Int(layout.byte_size() as i64))
        })
        .named_raw_function("align_of", &[], |cx, args| {
            let layout = type_arg_layout(cx, args, "align_of")?;
            Ok(Value::Int(layout.alignment() as i64))
        })
        .named_raw_function("from_bytes", &["data"], |cx, args| {
            let [Value::Bytes(data)] = args else {
                return Err(RuntimeError::new(
                    "`from_bytes` takes bytes: `from_bytes::<T>(data)`",
                ));
            };
            let layout = type_arg_layout(cx, &[], "from_bytes")?;
            layout.read(&lock(data), 0)
        })
        .named_raw_function("size_of_val", &["value"], |cx, args| {
            let [Value::Adt(adt)] = args else {
                return Err(RuntimeError::new("`size_of_val` takes a struct"));
            };
            Ok(Value::Int(layout_of_def(cx, adt.ctor, 0)?.byte_size() as i64))
        })
        .named_raw_function("to_bytes", &["value"], struct_to_bytes);
    registry
        .builtin(BuiltinType::Struct)
        .raw_function("to_bytes", struct_to_bytes)
        .raw_function("byte_size", |cx, args| {
            let [Value::Adt(adt)] = args else {
                return Err(RuntimeError::new("`byte_size` takes no arguments"));
            };
            Ok(Value::Int(layout_of_def(cx, adt.ctor, 0)?.byte_size() as i64))
        });
}

fn struct_to_bytes(cx: &mut dyn Context, args: &[Value]) -> RunResult<Value> {
    let [value @ Value::Adt(adt)] = args else {
        return Err(RuntimeError::new("`to_bytes` takes a struct"));
    };
    let layout = layout_of_def(cx, adt.ctor, 0)?;
    let mut buffer = vec![0; layout.byte_size()];
    layout.write(cx, &mut buffer, 0, value)?;
    Ok(Value::Bytes(Arc::new(buffer.into())))
}
