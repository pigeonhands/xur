//! `std::any`: reflection.

use std::sync::Arc;

use xur_runtime::{Context, Record, Registry, RunResult, RuntimeError, Value};
use xur_span::Symbol;

use crate::result;

pub(crate) fn register(registry: &Registry) {
    registry
        .module("std::any")
        .raw_function("type_name", type_name)
        .raw_function("try_cast", try_cast)
        .raw_function("record", to_record)
        .raw_function("type_name_val", type_name_val);
}

/// `type_name::<T>()`: the name of `T`.
fn type_name(cx: &mut dyn Context, args: &[Value]) -> RunResult<Value> {
    match (cx.type_args(), args) {
        ([ty], []) => Ok(Value::str(&ty.name)),
        _ => Err(RuntimeError::new(
            "`type_name` takes one type argument and no arguments: `type_name::<T>()`",
        )),
    }
}

/// `type_name_val(value)`: the type name of a value at runtime.
fn type_name_val(cx: &mut dyn Context, args: &[Value]) -> RunResult<Value> {
    match args {
        [value] => {
            let name = cx.describe_type(value);
            let name = name.strip_prefix('`').unwrap_or(&name);
            let name = name.strip_suffix('`').unwrap_or(name);
            Ok(Value::str(name))
        }
        _ => Err(RuntimeError::new(
            "`type_name_val` takes one argument: `type_name_val(v)`",
        )),
    }
}

/// `try_cast::<T>(value)`: attempt to coerce `value` into type `T`.
/// Returns `Ok(coerced)` on success or `Err(message)` on failure.
fn try_cast(cx: &mut dyn Context, args: &[Value]) -> RunResult<Value> {
    match (cx.type_args(), args) {
        ([ty], [value]) => {
            let def = ty.def.ok_or_else(|| {
                RuntimeError::new("`try_cast` requires a struct type argument: `try_cast::<T>(v)`")
            })?;
            match cx.try_cast(value, def) {
                Ok(coerced) => Ok(result(cx, Ok(coerced))),
                Err(e) => Ok(result(cx, Err(e.to_string()))),
            }
        }
        _ => Err(RuntimeError::new(
            "`try_cast` takes one type argument and one value: `try_cast::<T>(v)`",
        )),
    }
}

/// `record(value)`: convert a struct into an untyped record.
fn to_record(cx: &mut dyn Context, args: &[Value]) -> RunResult<Value> {
    match args {
        [value] => match value {
            Value::Record(_) => Ok(value.clone()),
            Value::Adt(adt) => {
                let info = cx.adt_info(adt.ctor).ok_or_else(|| {
                    RuntimeError::new(format!(
                        "cannot convert {} to a record",
                        cx.describe_type(value)
                    ))
                })?;
                let fields: Vec<_> = info
                    .fields
                    .iter()
                    .enumerate()
                    .map(|(i, fi)| {
                        let name = fi.name.unwrap_or_else(|| Symbol::intern(&i.to_string()));
                        let val = adt.fields.get(i).cloned().unwrap_or(Value::Unit);
                        (name, val)
                    })
                    .collect();
                Ok(Value::Record(Arc::new(Record::new(fields))))
            }
            _ => Err(RuntimeError::new(format!(
                "cannot convert {} to a record",
                cx.describe_type(value)
            ))),
        },
        _ => Err(RuntimeError::new(
            "`record` takes one argument: `record(value)`",
        )),
    }
}

#[cfg(test)]
mod tests {
    use xur_runtime::testing::{TestCx, fn_id};
    use xur_runtime::{TypeRef, Value};

    use crate::registry;

    #[test]
    fn type_name_uses_the_type_argument() {
        let registry = registry();
        let mut cx = TestCx::new(&registry);
        cx.type_args = vec![TypeRef::named("ZeroPacked")];
        assert_eq!(
            cx.call(fn_id(&registry, "std::any::type_name"), &[]),
            Ok(Value::str("ZeroPacked"))
        );
    }
}
