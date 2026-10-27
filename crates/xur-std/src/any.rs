//! `std::any`: reflection.

use xur_runtime::{Context, Registry, RunResult, RuntimeError, Value};

use crate::result;

pub(crate) fn register(registry: &Registry) {
    registry
        .module("std::any")
        .raw_function("type_name", type_name)
        .raw_function("try_cast", try_cast);
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
