//! `std::stub::godot` — Godot engine init and lifecycle for LD_PRELOAD game mods.
//!
//! Hooks the game's CoreCLR initialization and exposes it as a
//! `coreclr::Runtime`. Provides lifecycle pump setup and main-thread
//! dispatch for safely calling managed methods from other threads.
//!
//! ```text
//! use std::stub::godot;
//! use std::stub::coreclr;
//!
//! godot::intercept_process("game.dll", "Game.MainNode", "_Process");
//! let rt = godot::init(9)?;
//! godot::dispatch(|| { /* runs on game thread */ });
//! ```

use xur_runtime::convert::into_value;
use xur_runtime::{Registry, RuntimeError, Value};

use super::coreclr::{CoreClrRuntime, RUNTIME};

pub(crate) fn register(registry: &Registry) {
    registry.lazy_module("std::stub::godot", |m| {
        // -- init -----------------------------------------------------------

        m.raw_function("init", |cx, args| {
            let [Value::Int(major)] = args else {
                return Err(RuntimeError::new("`init` takes (major: int)"));
            };

            let (tx, rx) = std::sync::mpsc::sync_channel(1);
            let major = *major as u16;

            hao_godot::on_clr_init(move |host| {
                tracing::info!("godot: CLR init callback fired, sleeping 5s before attach");
                std::thread::sleep(std::time::Duration::from_secs(5));
                tracing::info!("godot: attaching to CoreCLR (major={major})");
                let result = host.attach(major);
                match &result {
                    Ok(_) => tracing::info!("godot: CoreCLR attach succeeded"),
                    Err(e) => tracing::error!("godot: CoreCLR attach failed: {e}"),
                }
                tx.send(result).ok();
            });

            let rt = rx
                .recv()
                .map_err(|_| RuntimeError::new("CLR init callback was dropped"))?
                .map_err(|e| RuntimeError::new(format!("CLR attach failed: {e}")))?;

            RUNTIME.set(rt).ok();

            let lang = cx.lang();
            let v = into_value(cx, CoreClrRuntime)?;
            Ok(Value::adt(lang.ok, vec![v]))
        });

        // -- lifecycle ------------------------------------------------------

        #[cfg(all(unix, target_arch = "x86_64"))]
        m.named_function(
            "intercept_process",
            &["assembly", "type_name", "method"],
            |assembly: String, type_name: String, method: String| {
                let lib = std::env::var("LD_PRELOAD")
                    .ok()
                    .and_then(|preload| {
                        preload
                            .split(':')
                            .find(|p| p.contains("xur_stub"))
                            .map(str::to_string)
                    })
                    .unwrap_or_else(|| "libxur_stub.so".into());

                hao_godot::lifecycle::intercept_process(&assembly, &type_name, &method, &lib);
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
        m.function("is_shutting_down", || hao_godot::lifecycle::is_shutting_down());

        // -- dispatch -------------------------------------------------------

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

            let result = hao_godot::dispatch::call(move || {
                handle.call_closure(&callback, &[])
            })
            .map_err(|e| RuntimeError::new(e.to_string()))?;

            result.map_err(|e| RuntimeError::new(e.message))
        });

        // -- scene ----------------------------------------------------------

        m.raw_function("scene_tree", |cx, _args| {
            let rt = RUNTIME
                .get()
                .ok_or_else(|| RuntimeError::new("runtime not initialized"))?;
            let main_loop = hao_godot::engine::engine::get_main_loop(rt)
                .map_err(|e| RuntimeError::new(e.to_string()))?;
            super::coreclr::managed_object_value(cx, main_loop as usize)
        });

        m.raw_function("root", |cx, _args| {
            let rt = RUNTIME
                .get()
                .ok_or_else(|| RuntimeError::new("runtime not initialized"))?;
            let main_loop = hao_godot::engine::engine::get_main_loop(rt)
                .map_err(|e| RuntimeError::new(e.to_string()))?;
            let root = hao_godot::engine::scene_tree::get_root(rt, main_loop as usize)
                .map_err(|e| RuntimeError::new(e.to_string()))?;
            super::coreclr::managed_object_value(cx, root as usize)
        });
    });
}
