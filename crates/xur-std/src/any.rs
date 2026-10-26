//! `std::any`: reflection.

use xur_runtime::{Context, Registry, RunResult, RuntimeError, Value};

pub(crate) fn register(registry: &Registry) {
    registry
        .module("std::any")
        .raw_function("type_name", type_name);
}

/// `type_name::<T>()`: the name of `T`.
fn type_name(cx: &mut dyn Context, args: &[Value]) -> RunResult<Value> {
    match (cx.type_args(), args) {
        ([name], []) => Ok(Value::str(name)),
        _ => Err(RuntimeError::new(
            "`type_name` takes one type argument and no arguments: `type_name::<T>()`",
        )),
    }
}

#[cfg(test)]
mod tests {
    use xur_runtime::Value;
    use xur_runtime::testing::{TestCx, fn_id};

    use crate::registry;

    #[test]
    fn type_name_uses_the_type_argument() {
        let registry = registry();
        let mut cx = TestCx::new(&registry);
        cx.type_args = vec!["ZeroPacked".into()];
        assert_eq!(
            cx.call(fn_id(&registry, "std::any::type_name"), &[]),
            Ok(Value::str("ZeroPacked"))
        );
    }
}
