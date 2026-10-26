//! Methods of integers and floats.

use xur_runtime::{BuiltinType, Registry};

pub(crate) fn register(registry: &mut Registry) {
    registry
        .builtin(BuiltinType::Int)
        .function("abs", |x: i64| x.abs())
        .function("pow", |x: i64, exp: u32| x.pow(exp))
        .function("min", |x: i64, y: i64| x.min(y))
        .function("max", |x: i64, y: i64| x.max(y))
        .function("to_float", |x: i64| x as f64)
        .function("to_string", |x: i64| x.to_string());
    registry
        .builtin(BuiltinType::Float)
        .function("abs", |x: f64| x.abs())
        .function("sqrt", |x: f64| x.sqrt())
        .function("floor", |x: f64| x.floor())
        .function("ceil", |x: f64| x.ceil())
        .function("round", |x: f64| x.round())
        .function("to_int", |x: f64| x as i64)
        .function("to_string", |x: f64| x.to_string());
}
