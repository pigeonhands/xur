//! `std::reflect`: looking inside values at runtime.
//!
//! Fields and methods can be listed, read and called by name, whatever the
//! value: a Xur struct or enum variant, a record, a tuple, a native object
//! or a built-in value.
//!
//! ```text
//! let p = Point { x: 1, y: 2 };
//! reflect::fields(p)              // ["x", "y"]
//! reflect::get(p, "x")            // 1
//! reflect::set(p, "x", 5)         // Point { x: 5, y: 2 }, a copy
//! reflect::call(p, "norm", [])    // p.norm()
//! ```

mod layout;

use xur_runtime::{Context, Registry, RunResult, RuntimeError, Value};

use crate::option;

pub(crate) fn register(registry: &Registry) {
    registry.lazy_module("std::reflect", |module| {
        module
            .raw_function("type_name", |cx, args| {
                let [value] = args else {
                    return arity("type_name", "value");
                };
                Ok(Value::str(&type_name(cx, value)))
            })
            .raw_function("fields", |cx, args| {
                let [value] = args else {
                    return arity("fields", "value");
                };
                Ok(names(fields(cx, value)))
            })
            .raw_function("has_field", |cx, args| {
                let [value, Value::Str(name)] = args else {
                    return arity("has_field", "value, name");
                };
                Ok(Value::Bool(fields(cx, value).iter().any(|f| f == &**name)))
            })
            .raw_function("get", |cx, args| {
                let [value, Value::Str(name)] = args else {
                    return arity("get", "value, name");
                };
                cx.field(value, name)
            })
            .raw_function("try_get", |cx, args| {
                let [value, Value::Str(name)] = args else {
                    return arity("try_get", "value, name");
                };
                let found = match fields(cx, value).iter().any(|f| f == &**name) {
                    true => Some(cx.field(value, name)?),
                    false => None,
                };
                Ok(option(cx, found))
            })
            .raw_function("set", |cx, args| {
                let [value, Value::Str(name), new] = args else {
                    return arity("set", "value, name, new_value");
                };
                cx.set_field(value, name, new.clone())
            })
            .raw_function("variant", |cx, args| {
                let [value] = args else {
                    return arity("variant", "value");
                };
                Ok(option(cx, variant(cx, value).map(|name| Value::str(&name))))
            })
            .raw_function("methods", |cx, args| {
                let [value] = args else {
                    return arity("methods", "value");
                };
                Ok(names(cx.methods(value)))
            })
            .raw_function("has_method", |cx, args| {
                let [value, Value::Str(name)] = args else {
                    return arity("has_method", "value, name");
                };
                Ok(Value::Bool(cx.methods(value).iter().any(|m| m == &**name)))
            })
            .raw_function("call", |cx, args| {
                let [receiver, Value::Str(name), Value::List(call_args)] = args else {
                    return arity("call", "value, name, [args]");
                };
                let call_args = xur_runtime::lock(call_args).clone();
                cx.call_method(receiver.clone(), name, call_args)
            });
        layout::register(module);
    });
}

/// The type of `value` by name: `Point`, `Shape::Circle`, `list`.
fn type_name(cx: &dyn Context, value: &Value) -> String {
    match value {
        Value::Adt(adt) => match cx.adt_info(adt.ctor) {
            Some(info) => info.name,
            None => cx.describe_type(value).trim_matches('`').to_owned(),
        },
        other => cx.describe_type(other).trim_matches('`').to_owned(),
    }
}

/// The variant of an enum value: `Circle` for `Shape::Circle(1.0)`.
fn variant(cx: &dyn Context, value: &Value) -> Option<String> {
    match value {
        Value::Record(record) => record.variant.map(|variant| variant.to_string()),
        Value::Adt(adt) => {
            let info = cx.adt_info(adt.ctor).filter(|info| info.is_variant)?;
            let (_, variant) = info.name.rsplit_once("::")?;
            Some(variant.to_owned())
        }
        _ => None,
    }
}

/// The names of the fields of `value`; numbers for tuples and tuple
/// structs.
fn fields(cx: &dyn Context, value: &Value) -> Vec<String> {
    match value {
        Value::Adt(adt) => cx
            .adt_info(adt.ctor)
            .map(|info| info.field_names())
            .unwrap_or_default(),
        Value::Record(record) => record
            .fields
            .iter()
            .map(|(name, _)| name.to_string())
            .collect(),
        Value::Tuple(items) => (0..items.len()).map(|i| i.to_string()).collect(),
        Value::Native(object) => cx.registry().type_fields(object.ty),
        _ => Vec::new(),
    }
}

fn names(names: Vec<String>) -> Value {
    Value::list(names.iter().map(|name| Value::str(name)).collect())
}

fn arity(name: &str, params: &str) -> RunResult<Value> {
    Err(RuntimeError::new(format!(
        "`reflect::{name}` takes ({params})"
    )))
}
