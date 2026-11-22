//! `std::xur` — information about the runtime environment.

use std::sync::OnceLock;

use xur_runtime::{Registry, RuntimeError, Value};

static INVOKED_BY: OnceLock<String> = OnceLock::new();
pub(crate) static ARGV: OnceLock<(Option<Value>, Vec<String>)> = OnceLock::new();

/// Sets how xur was invoked (e.g. "cli", "stub"). Must be called once
/// before any script runs; later calls are ignored.
pub fn set_invoked_by(name: &str) {
    let _ = INVOKED_BY.set(name.to_owned());
}

/// Sets the program's argument list. `script` is the path to the script
/// (`None` for stdin); `args` are the trailing arguments.
///
/// The script path is stored as `None` internally and wrapped in
/// `Option` when `std::argv()` is called (which has access to the
/// runtime's lang items).
pub fn set_argv(script: Option<String>, args: Vec<String>) {
    let script_value = script.map(|p| Value::str(&p));
    let _ = ARGV.set((script_value, args));
}

pub(crate) fn register(registry: &Registry) {
    registry
        .module("std::xur")
        .named_raw_function_returns::<String>("invoked_by", &[], |_cx, args| {
            if !args.is_empty() {
                return Err(RuntimeError::new("`invoked_by` takes no arguments"));
            }
            let name = INVOKED_BY.get().map_or("unknown", |s| s.as_str());
            Ok(Value::str(name))
        });
}
