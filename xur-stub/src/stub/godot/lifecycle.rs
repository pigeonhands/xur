//! Lifecycle pump and frame callbacks.

use xur_runtime::registry::ModuleBuilder;
use xur_runtime::{RuntimeError, Value};

pub(super) fn register(m: &mut ModuleBuilder<'_>) {
    m.function("hook_scene_tree", || {
        let lib = super::detect_lib_name();
        hao_godot::lifecycle::hook_scene_tree(&lib);
    });

    m.named_function(
        "intercept_process",
        &["assembly", "type_name", "method"],
        |assembly: String, type_name: String, method: String| {
            #[cfg(all(unix, target_arch = "x86_64"))]
            {
                let lib = super::detect_lib_name();
                hao_godot::lifecycle::intercept_process(&assembly, &type_name, &method, &lib);
            }
            #[cfg(not(all(unix, target_arch = "x86_64")))]
            {
                let _ = (&assembly, &type_name, &method);
            }
        },
    );

    m.raw_function("on_process", |cx, args| {
        let [callback] = args else {
            return Err(RuntimeError::new("`on_process` takes a callback"));
        };
        let callback = callback.clone();
        let handle: Box<xur_vm::SharedHandle> = cx
            .shared_handle()
            .and_then(|h| h.downcast().ok())
            .ok_or_else(|| RuntimeError::new("on_process requires VM support"))?;
        let handle = *handle;

        hao_godot::lifecycle::on_process(move || {
            if let Err(e) = handle.call_closure(&callback, &[]) {
                tracing::error!("godot on_process callback error: {}", e.message);
            }
        });
        Ok(Value::Unit)
    });

    m.function("shutdown", || hao_godot::lifecycle::shutdown());
    m.function("is_shutting_down", || {
        hao_godot::lifecycle::is_shutting_down()
    });
}
