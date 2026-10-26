//! `std::math`: plain Rust functions, registered with their facet types.

use xur_runtime::Registry;

pub(crate) fn register(registry: &mut Registry) {
    registry
        .module("std::math")
        .function("sqrt", |x: f64| x.sqrt())
        .function("pow", |x: f64, y: f64| x.powf(y))
        .function("abs", |x: f64| x.abs())
        .function("min", |a: i64, b: i64| a.min(b))
        .function("max", |a: i64, b: i64| a.max(b));
}

#[cfg(test)]
mod tests {
    use xur_runtime::Value;
    use xur_runtime::testing::{TestCx, fn_id};

    use crate::registry;

    #[test]
    fn math_functions_are_ordinary_rust_functions() {
        let registry = registry();
        let mut cx = TestCx::new(&registry);
        let sqrt = fn_id(&registry, "std::math::sqrt");
        assert_eq!(cx.call(sqrt, &[Value::Float(9.0)]), Ok(Value::Float(3.0)));
        let max = fn_id(&registry, "std::math::max");
        assert_eq!(
            cx.call(max, &[Value::Int(2), Value::Int(5)]),
            Ok(Value::Int(5))
        );
    }
}
