//! `std::assert`: test assertions.

use xur_runtime::{Context, Registry, RunResult, RuntimeError, Value};
use xur_runtime::registry::XurType;

pub(crate) fn register(registry: &Registry) {
    registry
        .module("std::assert")
        .named_raw_function_returns_xur("is_eq", &["left", "right"], XurType::UNIT, |cx, args| is_eq(cx, args))
        .named_raw_function_returns_xur("is_ne", &["left", "right"], XurType::UNIT, |cx, args| is_ne(cx, args))
        .named_raw_function_returns_xur("is_true", &["value"], XurType::UNIT, |_, args| is_true(args))
        .named_raw_function_returns_xur("is_false", &["value"], XurType::UNIT, |_, args| is_false(args))
        .named_raw_function_returns_xur("fail", &["message"], XurType::UNIT, |cx, args| fail(cx, args));
}

fn coerce_repr(cx: &dyn Context, value: &Value) -> Option<Value> {
    if let Value::Adt(adt) = value
        && adt.fields.is_empty()
    {
        return cx.repr_value(adt.ctor).map(Value::int);
    }
    None
}

fn values_eq(cx: &dyn Context, left: &Value, right: &Value) -> bool {
    match (left, right) {
        (Value::Native(a), Value::Native(b)) => cx.registry().native_eq(a, b),
        _ => left == right,
    }
}

fn repr_eq(cx: &dyn Context, left: &Value, right: &Value) -> bool {
    let lc = coerce_repr(cx, left);
    let rc = coerce_repr(cx, right);
    match (&lc, &rc) {
        (Some(_), _) | (_, Some(_)) => {
            let left = lc.as_ref().unwrap_or(left);
            let right = rc.as_ref().unwrap_or(right);
            values_eq(cx, left, right)
        }
        _ => values_eq(cx, left, right),
    }
}

fn is_eq(cx: &mut dyn Context, args: &[Value]) -> RunResult<Value> {
    let [left, right] = args else {
        return Err(RuntimeError::new("`is_eq` takes two arguments"));
    };
    if !repr_eq(cx, left, right) {
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
    if repr_eq(cx, left, right) {
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
