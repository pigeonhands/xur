use xur_runtime::{Registry, RuntimeError, Value};

pub(crate) fn register(registry: &Registry) {
    registry.lazy_module("std::stub::re::unity::time", |m| {
        use hao_il2cpp_unity::time;

        m.raw_function("scale", |_cx, args| {
            if !args.is_empty() {
                return Err(RuntimeError::new("`scale` takes no arguments"));
            }
            let scale = time::scale().unwrap_or(1.0);
            Ok(Value::Float(scale as f64))
        });

        m.raw_function("set_scale", |_cx, args| {
            let [Value::Float(scale)] = args else {
                return Err(RuntimeError::new("`set_scale` takes a float"));
            };
            let _ = time::set_scale(*scale as f32);
            Ok(Value::Unit)
        });

        m.raw_function("delta", |_cx, args| {
            if !args.is_empty() {
                return Err(RuntimeError::new("`delta` takes no arguments"));
            }
            let dt = time::delta().unwrap_or(0.0);
            Ok(Value::Float(dt as f64))
        });

        m.raw_function("frame_count", |_cx, args| {
            if !args.is_empty() {
                return Err(RuntimeError::new("`frame_count` takes no arguments"));
            }
            let count = time::frame_count().unwrap_or(0);
            Ok(Value::Int(count as i64))
        });
    });
}
