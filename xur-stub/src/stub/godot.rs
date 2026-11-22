//! `std::stub::godot` — Godot engine init for LD_PRELOAD game mods.
//!
//! Hooks the game's CoreCLR initialization and exposes it as a
//! `coreclr::Runtime`. All generic CoreCLR types (`MethodPtr`,
//! `ManagedObject`, etc.) live in `std::stub::coreclr`.
//!
//! ```text
//! use std::stub::godot;
//! use std::stub::coreclr;
//!
//! let rt = godot::init(9)?;
//! let method = rt.method("sts2", "Some.Type", "get_Value")?;
//! ```

use xur_runtime::convert::into_value;
use xur_runtime::{Registry, RuntimeError, Value};

use super::coreclr::{CoreClrRuntime, RUNTIME};

pub(crate) fn register(registry: &Registry) {
    registry.lazy_module("std::stub::godot", |m| {
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
    });
}
