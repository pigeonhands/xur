//! `std::math`: plain Rust functions, registered with their facet types.

use xur_runtime::Registry;

pub(crate) fn register(registry: &Registry) {
    registry
        .module("std::math")
        .named_function("sqrt", &["x"], |x: f64| x.sqrt())
        .named_function("pow", &["base", "exp"], |x: f64, y: f64| x.powf(y))
        .named_function("abs", &["x"], |x: f64| x.abs())
        .named_function("min", &["a", "b"], |a: i64, b: i64| a.min(b))
        .named_function("max", &["a", "b"], |a: i64, b: i64| a.max(b))
        // Trigonometric functions
        .named_function("sin", &["x"], |x: f64| x.sin())
        .named_function("cos", &["x"], |x: f64| x.cos())
        .named_function("tan", &["x"], |x: f64| x.tan())
        .named_function("asin", &["x"], |x: f64| x.asin())
        .named_function("acos", &["x"], |x: f64| x.acos())
        .named_function("atan", &["x"], |x: f64| x.atan())
        .named_function("atan2", &["y", "x"], |y: f64, x: f64| y.atan2(x))
        // Logarithmic / exponential
        .named_function("ln", &["x"], |x: f64| x.ln())
        .named_function("log2", &["x"], |x: f64| x.log2())
        .named_function("log10", &["x"], |x: f64| x.log10())
        .named_function("exp", &["x"], |x: f64| x.exp())
        // Rounding
        .named_function("floor", &["x"], |x: f64| x.floor())
        .named_function("ceil", &["x"], |x: f64| x.ceil())
        .named_function("round", &["x"], |x: f64| x.round())
        // Constants (zero-arg functions)
        .named_function("PI", &[], || std::f64::consts::PI)
        .named_function("E", &[], || std::f64::consts::E)
        .named_function("TAU", &[], || std::f64::consts::TAU)
        .named_function("INFINITY", &[], || f64::INFINITY)
        .named_function("NAN", &[], || f64::NAN)
        // Utility
        .named_function("clamp", &["val", "min", "max"], |x: f64, min: f64, max: f64| x.clamp(min, max))
        .named_function("lerp", &["a", "b", "t"], |a: f64, b: f64, t: f64| a + (b - a) * t)
        .named_function("fmin", &["a", "b"], |a: f64, b: f64| a.min(b))
        .named_function("fmax", &["a", "b"], |a: f64, b: f64| a.max(b));
}

#[cfg(test)]
mod tests {
    use xur_runtime::Value;
    use xur_runtime::testing::{TestCx, fn_id};

    use crate::registry;

    fn call(name: &str, args: &[Value]) -> Value {
        let registry = registry();
        let mut cx = TestCx::new(&registry, crate::PRELUDE);
        let id = fn_id(&registry, &format!("std::math::{name}"));
        cx.call(id, args).unwrap()
    }

    fn approx_eq(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-10
    }

    #[test]
    fn math_functions_are_ordinary_rust_functions() {
        let registry = registry();
        let mut cx = TestCx::new(&registry, crate::PRELUDE);
        let sqrt = fn_id(&registry, "std::math::sqrt");
        assert_eq!(cx.call(sqrt, &[Value::Float(9.0)]), Ok(Value::Float(3.0)));
        let max = fn_id(&registry, "std::math::max");
        assert_eq!(
            cx.call(max, &[Value::Int(2), Value::Int(5)]),
            Ok(Value::Int(5))
        );
    }

    #[test]
    fn trig_functions() {
        let pi = std::f64::consts::PI;
        let Value::Float(sin_half_pi) = call("sin", &[Value::Float(pi / 2.0)]) else {
            panic!("expected float");
        };
        assert!(approx_eq(sin_half_pi, 1.0));

        let Value::Float(cos_zero) = call("cos", &[Value::Float(0.0)]) else {
            panic!("expected float");
        };
        assert!(approx_eq(cos_zero, 1.0));

        let Value::Float(atan2_val) = call("atan2", &[Value::Float(1.0), Value::Float(1.0)]) else {
            panic!("expected float");
        };
        assert!(approx_eq(atan2_val, pi / 4.0));
    }

    #[test]
    fn log_functions() {
        let e = std::f64::consts::E;
        let Value::Float(ln_e) = call("ln", &[Value::Float(e)]) else {
            panic!("expected float");
        };
        assert!(approx_eq(ln_e, 1.0));

        let Value::Float(log2_8) = call("log2", &[Value::Float(8.0)]) else {
            panic!("expected float");
        };
        assert!(approx_eq(log2_8, 3.0));

        let Value::Float(exp_1) = call("exp", &[Value::Float(1.0)]) else {
            panic!("expected float");
        };
        assert!(approx_eq(exp_1, e));
    }

    #[test]
    fn rounding_functions() {
        assert_eq!(call("floor", &[Value::Float(2.7)]), Value::Float(2.0));
        assert_eq!(call("ceil", &[Value::Float(2.1)]), Value::Float(3.0));
        assert_eq!(call("round", &[Value::Float(2.5)]), Value::Float(3.0));
    }

    #[test]
    fn constants() {
        let Value::Float(pi) = call("PI", &[]) else {
            panic!("expected float");
        };
        assert!(approx_eq(pi, std::f64::consts::PI));

        let Value::Float(e) = call("E", &[]) else {
            panic!("expected float");
        };
        assert!(approx_eq(e, std::f64::consts::E));

        let Value::Float(tau) = call("TAU", &[]) else {
            panic!("expected float");
        };
        assert!(approx_eq(tau, std::f64::consts::TAU));

        let Value::Float(inf) = call("INFINITY", &[]) else {
            panic!("expected float");
        };
        assert!(inf.is_infinite() && inf.is_sign_positive());

        let Value::Float(nan) = call("NAN", &[]) else {
            panic!("expected float");
        };
        assert!(nan.is_nan());
    }

    #[test]
    fn utility_functions() {
        assert_eq!(
            call("clamp", &[Value::Float(5.0), Value::Float(0.0), Value::Float(3.0)]),
            Value::Float(3.0)
        );

        let Value::Float(lerp_val) = call("lerp", &[Value::Float(0.0), Value::Float(10.0), Value::Float(0.5)]) else {
            panic!("expected float");
        };
        assert!(approx_eq(lerp_val, 5.0));

        assert_eq!(
            call("fmin", &[Value::Float(3.0), Value::Float(1.0)]),
            Value::Float(1.0)
        );
        assert_eq!(
            call("fmax", &[Value::Float(3.0), Value::Float(1.0)]),
            Value::Float(3.0)
        );
    }
}
