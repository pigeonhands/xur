//! `std::xur` — information about the runtime environment.

use std::sync::OnceLock;

use xur_runtime::{Registry, RuntimeError, Value};

static INVOKED_BY: OnceLock<String> = OnceLock::new();

/// Sets how xur was invoked (e.g. "cli", "stub"). Must be called once
/// before any script runs; later calls are ignored.
pub fn set_invoked_by(name: &str) {
    let _ = INVOKED_BY.set(name.to_owned());
}

pub(crate) fn register(registry: &Registry) {
    registry
        .module("std::xur")
        .raw_function("invoked_by", |_cx, args| {
            if !args.is_empty() {
                return Err(RuntimeError::new("`invoked_by` takes no arguments"));
            }
            let name = INVOKED_BY.get().map_or("unknown", |s| s.as_str());
            Ok(Value::str(name))
        });
}
