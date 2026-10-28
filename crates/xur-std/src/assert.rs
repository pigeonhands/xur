//! `std::assert`: test assertions.

use xur_runtime::{Context, Registry, RunResult, RuntimeError, Value};

pub(crate) fn register(registry: &Registry) {
    registry
        .module("std::assert")
        .named_raw_function("is_eq", &["left", "right"], |cx, args| is_eq(cx, args))
        .named_raw_function("is_ne", &["left", "right"], |cx, args| is_ne(cx, args))
        .named_raw_function("is_true", &["value"], |_, args| is_true(args))
        .named_raw_function("is_false", &["value"], |_, args| is_false(args))
        .named_raw_function("fail", &["message"], |cx, args| fail(cx, args));
}

fn is_eq(cx: &mut dyn Context, args: &[Value]) -> RunResult<Value> {
    let [left, right] = args else {
        return Err(RuntimeError::new("`is_eq` takes two arguments"));
    };
    if left != right {
        return Err(RuntimeError::new(format!(
            "assertion failed: {} != {}",
            cx.display(left),
            cx.display(right),
        )));
    }
    Ok(Value::Unit)
}

fn is_ne(cx: &mut dyn Context, args: &[Value]) -> RunResult<Value> {
    let [left, right] = args else {
        return Err(RuntimeError::new("`is_ne` takes two arguments"));
    };
    if left == right {
        return Err(RuntimeError::new(format!(
            "assertion failed: values are equal: {}",
            cx.display(left),
        )));
    }
    Ok(Value::Unit)
}

fn is_true(args: &[Value]) -> RunResult<Value> {
    match args {
        [Value::Bool(true)] => Ok(Value::Unit),
        [Value::Bool(false)] => Err(RuntimeError::new("assertion failed: expected true")),
        _ => Err(RuntimeError::new("`is_true` takes a bool")),
    }
}

fn is_false(args: &[Value]) -> RunResult<Value> {
    match args {
        [Value::Bool(false)] => Ok(Value::Unit),
        [Value::Bool(true)] => Err(RuntimeError::new("assertion failed: expected false")),
        _ => Err(RuntimeError::new("`is_false` takes a bool")),
    }
}

fn fail(cx: &mut dyn Context, args: &[Value]) -> RunResult<Value> {
    match args {
        [Value::Str(msg)] => Err(RuntimeError::new(format!("assertion failed: {}", &**msg))),
        [] => Err(RuntimeError::new("assertion failed")),
        [value] => Err(RuntimeError::new(format!(
            "assertion failed: {}",
            cx.display(value)
        ))),
        _ => Err(RuntimeError::new("`fail` takes an optional message")),
    }
}
