//! Main-thread dispatch — queue or synchronously call closures on the game thread.

use xur_runtime::registry::ModuleBuilder;
use xur_runtime::{RuntimeError, Value};

pub(super) fn register(m: &mut ModuleBuilder<'_>) {
    m.raw_function("dispatch", |cx, args| {
        let [callback] = args else {
            return Err(RuntimeError::new("`dispatch` takes a callback"));
        };
        let callback = callback.clone();
        let handle: Box<xur_vm::SharedHandle> = cx
            .shared_handle()
            .and_then(|h| h.downcast().ok())
            .ok_or_else(|| RuntimeError::new("dispatch requires VM support"))?;
        let handle = *handle;

        hao_godot::dispatch::queue(move || {
            if let Err(e) = handle.call_closure(&callback, &[]) {
                tracing::error!("godot dispatch callback error: {}", e.message);
            }
        })
        .map_err(|e| RuntimeError::new(e.to_string()))?;
        Ok(Value::Unit)
    });

    m.raw_function("dispatch_call", |cx, args| {
        let [callback] = args else {
            return Err(RuntimeError::new("`dispatch_call` takes a callback"));
        };
        let callback = callback.clone();
        let handle: Box<xur_vm::SharedHandle> = cx
            .shared_handle()
            .and_then(|h| h.downcast().ok())
            .ok_or_else(|| RuntimeError::new("dispatch_call requires VM support"))?;
        let handle = *handle;

        let result = hao_godot::dispatch::call(move || handle.call_closure(&callback, &[]))
            .map_err(|e| RuntimeError::new(e.to_string()))?;

        result.map_err(|e| RuntimeError::new(e.message))
    });
}
