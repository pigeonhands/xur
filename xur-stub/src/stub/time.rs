use facet::Facet;
use xur_runtime::Registry;

#[derive(Facet)]
#[repr(C)]
#[allow(dead_code)]
enum TimeError {
    SetScale(String),
}

pub(crate) fn register(registry: &Registry) {
    registry.lazy_module("std::stub::re::unity::time", |m| {
        use hao_il2cpp_unity::time;

        m.shape_type("TimeError", TimeError::SHAPE);

        m.function("scale", || -> f64 { time::scale().unwrap_or(1.0) as f64 });

        m.named_function("set_scale", &["scale"], |scale: f64| -> Result<(), TimeError> {
            time::set_scale(scale as f32).map_err(|e| TimeError::SetScale(e.to_string()))
        });

        m.function("delta", || -> f64 { time::delta().unwrap_or(0.0) as f64 });

        m.function("frame_count", || -> i64 {
            time::frame_count().unwrap_or(0) as i64
        });
    });
}
